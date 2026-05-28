//! Один OS-thread = одна сессия = один MoonBot-сервер.
//!
//! Учитывает `servers.modules` JSONB: если `listener_strategies` выключен —
//! StrategySnapshot не пишется, аналогично для `listener_orders`. Это позволяет
//! на одном сервере держать только то что нужно (например, только стратегии
//! для UI-отчётов).

use anyhow::Context;
use moonproto::state::{OrderEvent, StratEvent};
use moonproto::{
    parse_key_info, ClientConfig, ConnectConfig, Event, InitConfig, InitialStrategies, MoonClient,
};

use crate::config::DbServer;
use crate::{db, settings, storage};

pub fn run_session(server: &DbServer, db_url: &str) -> anyhow::Result<()> {
    let mut sql = db::connect(db_url).context("DB connect failed")?;

    let info = parse_key_info(&server.token).ok_or_else(|| {
        anyhow::anyhow!(
            "[{}] token is not a valid MoonProto base64 key (length={})",
            server.name, server.token.len()
        )
    })?;

    let host = server.ip.clone();
    let port = server.port as u16;

    let mask_ver = info.network.map(|n| n.mask_ver).unwrap_or(0);
    let client_cfg =
        ClientConfig::new(host.clone(), port, info.keys.master_key, info.keys.mac_key)
            .with_transport_mode(mask_ver);

    let init = InitConfig {
        initial_strategies: Some(InitialStrategies::new(0, Vec::new())),
        step_timeout: None,
        ..Default::default()
    };

    tracing::info!(
        server = %server.name,
        modules = ?server.modules,
        "connecting to MoonBot via MoonProto"
    );
    let client = MoonClient::connect(
        client_cfg,
        ConnectConfig::new(init).with_connect_timeout(settings::connect_timeout()),
    )
    .with_context(|| format!("[{}] connect/init failed", server.name))?;

    tracing::info!("[{}] MoonProto init done", server.name);

    // Первичный sync
    initial_sync(&mut sql, &client, server)?;

    tracing::info!("[{}] entering event loop", server.name);
    loop {
        match client.recv_event_timeout(settings::poll_interval()) {
            Ok(event) => {
                if let Err(e) = handle_event(&mut sql, &client, server, &event) {
                    tracing::warn!("[{}] event handler error: {e:#}", server.name);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                tracing::warn!("[{}] client runtime stopped, exiting session", server.name);
                break;
            }
        }
    }
    Ok(())
}

fn initial_sync(
    sql: &mut postgres::Client,
    client: &MoonClient,
    server: &DbServer,
) -> anyhow::Result<()> {
    let Some(snap) = client.snapshot() else {
        tracing::warn!("[{}] no snapshot available after init", server.name);
        return Ok(());
    };

    if server.modules.listener_strategies {
        let strats = snap.strategy_snapshot_vec();
        let mut ok = 0usize;
        for s in &strats {
            match storage::strategies::upsert_snapshot(sql, server.id, s) {
                Ok(_) => ok += 1,
                Err(e) => tracing::warn!("[{}] strategies upsert failed: {e:#}", server.name),
            }
        }
        tracing::info!("[{}] initial strategies synced: {}/{}", server.name, ok, strats.len());
    }

    if server.modules.listener_orders {
        let mut uids = Vec::new();
        let mut ok = 0usize;
        for order in snap.orders().iter() {
            uids.push(order.uid);
            match storage::orders::upsert(sql, server.id, order) {
                Ok(_) => ok += 1,
                Err(e) => tracing::warn!("[{}] orders upsert failed (uid={}): {e:#}", server.name, order.uid),
            }
        }
        if settings::get_bool("orders_sync_on_snapshot", true) {
            let _ = storage::orders::sync_snapshot(sql, server.id, uids.clone());
        }
        tracing::info!("[{}] initial orders synced: {}/{}", server.name, ok, uids.len());
    }
    Ok(())
}

fn handle_event(
    sql: &mut postgres::Client,
    client: &MoonClient,
    server: &DbServer,
    event: &Event,
) -> anyhow::Result<()> {
    match event {
        Event::Strat(StratEvent::SchemaApplied { kind_count, field_count, .. }) => {
            tracing::info!(
                "[{}] strategy schema applied: kinds={kind_count} fields={field_count}",
                server.name
            );
        }
        Event::Strat(_) if server.modules.listener_strategies => {
            if let Some(snap) = client.snapshot() {
                for s in snap.strategy_snapshot_vec() {
                    storage::strategies::upsert_snapshot(sql, server.id, &s)?;
                }
            }
        }
        Event::Order(OrderEvent::Created(uid)) | Event::Order(OrderEvent::Updated(uid))
            if server.modules.listener_orders =>
        {
            if let Some(snap) = client.snapshot() {
                if let Some(order) = snap.orders().get(*uid) {
                    storage::orders::upsert(sql, server.id, order)?;
                }
            }
        }
        Event::Order(OrderEvent::Removed(uid)) if server.modules.listener_orders => {
            storage::orders::delete(sql, server.id, *uid)?;
        }
        Event::Order(OrderEvent::Snapshot) if server.modules.listener_orders => {
            if let Some(snap) = client.snapshot() {
                let mut uids = Vec::new();
                for order in snap.orders().iter() {
                    uids.push(order.uid);
                    storage::orders::upsert(sql, server.id, order)?;
                }
                if settings::get_bool("orders_sync_on_snapshot", true) {
                    let removed = storage::orders::sync_snapshot(sql, server.id, uids)?;
                    if removed > 0 {
                        tracing::debug!("[{}] orders snapshot removed {removed} stale rows", server.name);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
