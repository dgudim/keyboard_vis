use std::error::Error;
use std::sync::atomic::Ordering;

use log::{info, warn};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    Event as IdleNotificationEvent, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use crate::consts::{IDLE_TIMEOUT_MS, USER_IDLE};

struct AppState {
    _seat: WlSeat,
    _notifier: ExtIdleNotifierV1,
    _notification: ExtIdleNotificationV1,
}

delegate_noop!(AppState: ignore WlSeat);
delegate_noop!(AppState: ignore ExtIdleNotifierV1);

impl Dispatch<WlRegistry, GlobalListContents> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: IdleNotificationEvent,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            IdleNotificationEvent::Idled => {
                info!("Wayland: user idle");
                USER_IDLE.store(true, Ordering::Relaxed);
            }
            IdleNotificationEvent::Resumed => {
                info!("Wayland: user active");
                USER_IDLE.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

fn run_wayland_monitor() -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<AppState>(&conn)?;
    let qh = event_queue.handle();

    let seat: WlSeat = globals.bind(&qh, 1..=9, ())?;
    let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=2, ())?;
    let notification = notifier.get_input_idle_notification(IDLE_TIMEOUT_MS, &seat, &qh, ());

    let mut state = AppState {
        _seat: seat,
        _notifier: notifier,
        _notification: notification,
    };

    info!("Wayland idle monitor started ({IDLE_TIMEOUT_MS}ms timeout)");

    loop {
        event_queue.blocking_dispatch(&mut state)?;
    }
}

pub fn spawn_wayland_monitor() {
    std::thread::spawn(|| {
        if let Err(e) = run_wayland_monitor() {
            warn!("Wayland monitor unavailable: {e}");
        }
    });
}
