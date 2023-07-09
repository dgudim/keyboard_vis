use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH}, vec,
};

use crate::utils::{
    display_progress, fade_into_frame, parse_hex, Frame, Notification, ProgressMap,
    BLACK_SUBSTRATE, BLUE, GREEN, PURPLE, RED, TOP_ROW_LENGTH, WHITE,
};
use dbus::{
    arg::{prop_cast, PropMap},
    blocking::Connection,
    channel::MatchingReceiver,
    message::MatchRule,
    Message,
};

fn get_full_match_rule<'a>(interface: &'a str, path: &'a str, member: &'a str) -> MatchRule<'a> {
    return MatchRule::with_member(
        MatchRule::with_interface(MatchRule::with_path(MatchRule::new(), path), interface),
        member,
    );
}

pub fn process_dbus(base_frame: Frame) -> Result<(), Box<dyn Error>> {
    let screen_locked = Arc::new(AtomicBool::new(false));

    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let conn = Connection::new_session()?;

    let pending_notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));
    let notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));

    let notification_map = vec!["Thunderbird", "Telegram desktop"];

    let mut progress_map = ProgressMap::new();
    progress_map.insert(
        "application://firefox".to_string(),
        (parse_hex("#ff4503"), 0.0),
    );
    progress_map.insert(
        "application://org.kde.dolphin".to_string(),
        (parse_hex("#03c2fc"), 0.0),
    );

    let mr_progress = MatchRule::new_signal("com.canonical.Unity.LauncherEntry", "Update");
    let mr_screen = get_full_match_rule(
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "ActiveChanged",
    );
    let mr_notification_closed = get_full_match_rule(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "NotificationClosed",
    );

    let mr_notification_opened = get_full_match_rule(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "Notify",
    );

    let dbus_proxy = conn.with_proxy(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        Duration::from_millis(5000),
    );

    let (notification_server_name,): (String,) = dbus_proxy.method_call(
        "org.freedesktop.DBus",
        "GetNameOwner",
        ("org.freedesktop.Notifications",),
    )?;

    let mr_notification_delivered = MatchRule::with_sender(
        MatchRule::with_type(MatchRule::new(), dbus::MessageType::MethodReturn),
        notification_server_name,
    );

    // become monitor, match all the necessary methods/signals
    dbus_proxy.method_call(
        "org.freedesktop.DBus.Monitoring",
        "BecomeMonitor",
        (
            vec![
                mr_progress.match_str(),
                mr_screen.match_str(),
                mr_notification_closed.match_str(),
                mr_notification_opened.match_str(),
                mr_notification_delivered.match_str(),
            ],
            0u32,
        ),
    )?;

    let screen_locked_p = screen_locked.clone();
    let base_frame_p = base_frame.clone();
    conn.start_receive(
        mr_progress,
        Box::new(move |message: Message, _| {
            let (source, props): (&str, PropMap) = message.read2().unwrap();
            if !props.contains_key("progress") {
                true;
            }

            let progress: f64 = *prop_cast(&props, "progress").unwrap_or(&0.0);
            let progress_visible: bool = *prop_cast(&props, "progress-visible").unwrap_or(&true);
            let count: i32 = *prop_cast(&props, "count").unwrap_or(&0);

            // flash in special cases
            if progress == 0.0 {
                let frame_to = base_frame_p
                    .clone()
                    .iter()
                    .enumerate()
                    .map(|(index, val)| {
                        if index < TOP_ROW_LENGTH as usize {
                            return if progress_visible {
                                if count > 1 {
                                    *GREEN // visible notification with visible progress that has (just started probably)
                                } else {
                                    *RED // invisible (probably closed) notification with visible progress (idk when this occurs)
                                }
                            } else {
                                if count > 1 {
                                    *BLUE // visible notification without visible progress (idk when this occurs)
                                } else {
                                    *PURPLE // invisible notification without visible progress (spectacle call, download finished)
                                }
                            };
                        }
                        return *val;
                    })
                    .collect();
                fade_into_frame(&frame_to, 300);
                fade_into_frame(&base_frame_p, 600);
            }

            let mut tuple = progress_map
                .entry(source.to_string())
                .or_insert((WHITE, 0.0));
            tuple.1 = progress;

            display_progress(
                if screen_locked_p.load(Ordering::Relaxed) {
                    &BLACK_SUBSTRATE
                } else {
                    &base_frame_p
                },
                &progress_map,
            );

            println!("Notification progress for {source} = {progress}");
            true
        }),
    );

    conn.start_receive(
        mr_screen,
        Box::new(move |message: Message, _| {
            let locked: bool = message.read1().unwrap();
            println!("Screen locked/unlocked: {locked}");
            screen_locked.store(locked, Ordering::Relaxed);
            fade_into_frame(
                if locked {
                    &BLACK_SUBSTRATE
                } else {
                    &base_frame
                },
                1500,
            );
            true
        }),
    );

    let pending_notification_qc = pending_notification_q.clone();
    conn.start_receive(
        mr_notification_opened,
        Box::new(move |message: Message, _| {
            let (source, _, _, summary): (String, u32, String, String) = message.read4().unwrap();
            let sender = message.sender().unwrap().to_string();
            println!("Notification sent from {source} ({sender}) | {summary}");
            let mut pending_notif_q = pending_notification_qc.write().unwrap();

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_millis();

            pending_notif_q.push(Notification {
                id: 0,
                source,
                color: *RED,
                timestamp,
            });
            true
        }),
    );

    conn.start_receive(
        mr_notification_closed,
        Box::new(|message: Message, _| {
            let (id, reason): (u32, u32) = message.read2().unwrap();
            println!("Notification closed {id} with reason {reason}");
            true
        }),
    );

    conn.start_receive(
        mr_notification_delivered,
        Box::new(|message: Message, _| {
            match message.read1::<u32>() {
                Ok(id) => {
                    println!(
                        "Notification delivered {:#?} (reply to {})",
                        id,
                        message.destination().unwrap()
                    );
                }
                Err(e) => {
                    println!("Unknown message: {:?}: {e}", message)
                }
            };
            true
        }),
    );

    loop {
        conn.process(Duration::from_millis(1000)).unwrap();
    }
}
