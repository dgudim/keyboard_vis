use std::{
    collections::HashMap,
    error::Error,
    sync::{
        atomic::Ordering,
        Arc, RwLock,
    },
    time::Duration,
    vec,
};

use crate::utils::{
    display_notifications, display_progress, fade_into_frame, flash_frame, get_frame_by_condition,
    get_timestamp, num2xy, parse_hex, Frame, Notification, NotificationSettings, ProgressMap,
    BLACK, BLACK_SUBSTRATE, BLUE, GREEN, PURPLE, RED, ROW_LENGTH, TOP_ROW_LENGTH, WHITE, SCREEN_LOCKED, COL_OFFSET, CYAN,
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

    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let conn = Connection::new_session()?;

    let pending_notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));
    let notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));

    let mut notification_map = HashMap::new();
    notification_map.insert(
        "Thunderbird",
        Arc::new(NotificationSettings {
            color: *BLUE,
            flash_on_auto_close: *CYAN,
            flash_on_notify: false,
            important: true,
        }),
    );
    notification_map.insert(
        "Telegram Desktop",
        Arc::new(NotificationSettings {
            color: WHITE,
            flash_on_auto_close: *BLUE,
            flash_on_notify: true,
            important: true,
        }),
    );
    notification_map.insert(
        "notify-send",
        Arc::new(NotificationSettings {
            color: *RED,
            flash_on_auto_close: *BLUE,
            flash_on_notify: true,
            important: true,
        }),
    );
    
    let notification_delivery_timeout = 2000;

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
                let frame_to = get_frame_by_condition(&base_frame_p, &|(index, val)| {
                    if index < (TOP_ROW_LENGTH + COL_OFFSET) as usize {
                        return if progress_visible {
                            if count > 1 {
                                *GREEN // visible notification with visible progress that has (just started probably)
                            } else {
                                *RED // invisible (probably closed) notification with visible progress (idk when this occurs, probably not even on notifications)
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
                });
                flash_frame(&frame_to, &base_frame_p, 200, 300, 200);
            }

            let mut tuple = progress_map
                .entry(source.to_string())
                .or_insert((WHITE, 0.0));
            tuple.1 = progress;

            display_progress(&base_frame_p, &progress_map);

            println!("Notification progress for {source} = {progress}");
            true
        }),
    );

    let base_frame_p = base_frame.clone();
    let screen_locked_p = SCREEN_LOCKED.clone();
    conn.start_receive(
        mr_screen,
        Box::new(move |message: Message, _| {
            let locked: bool = message.read1().unwrap();
            println!("Screen locked/unlocked: {locked}");
            screen_locked_p.store(locked, Ordering::Relaxed);
            fade_into_frame(
                if locked {
                    &BLACK_SUBSTRATE
                } else {
                    &base_frame_p
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
            let (application, _, _, summary): (String, u32, String, String) =
                message.read4().unwrap();
            let sender = message.sender().unwrap().to_string();
            println!("Notification sent from {application} ({sender}) | {summary}");
            let mut pending_notif_q = pending_notification_qc.write().unwrap();

            match notification_map
                .get(application.as_str()) {
                    Some(arc_settings) => {
                        pending_notif_q.push(Notification {
                            id: 0,
                            sender,
                            timestamp: get_timestamp(),
                            settings: arc_settings.clone(),
                        });
                    },
                    None => println!("Notification isn't in the map, ignoring"),
                };

            
            true
        }),
    );

    let find_in_notif_q = |id: u32, notif_q: &Vec<Notification>| -> i64 {
        return notif_q
            .iter()
            .enumerate()
            .find(|(_, notif)| notif.id == id)
            .map_or(-1, |(index, _)| index as i64);
    };

    let pending_notification_qc = pending_notification_q.clone();
    let base_frame_p = base_frame.clone();
    conn.start_receive(
        mr_notification_closed,
        Box::new(move |message: Message, _| {
            let (id, reason): (u32, u32) = message.read2().unwrap();

            let mut pending_notif_q = pending_notification_qc.write().unwrap();
            let mut notif_q = notification_q.write().unwrap();

            let ind: i64 = find_in_notif_q(id, &pending_notif_q);

            if ind != -1 {
                let notif = pending_notif_q.remove(ind as usize);

                if reason == 2 {
                    println!(" -=-=- Pending notification closed by user, id: {id}");
                } else {
                    println!(" -=-=- Pending notification closed automatically, id: {id}");

                    let settings = &notif.settings;

                    if settings.flash_on_auto_close != BLACK {
                        // reason = 1 - expired, 2 - user, 3 - auto, 4 - other
                        let frame = get_frame_by_condition(&base_frame_p, &|(index, color)| {
                            let pos = num2xy(index);
                            if pos.x >= ROW_LENGTH - 4 {
                                settings.flash_on_auto_close
                            } else {
                                *color
                            }
                        });
                        flash_frame(&frame, &base_frame_p, 300, 1000, 300);
                    }

                    if settings.important {
                        notif_q.push(notif);
                        println!("Moved pending notification {id} to display queue");
                        display_notifications(&base_frame_p, &notif_q);
                    }
                }
                return true
            }

            let ind_full: i64 = find_in_notif_q(id, &notif_q);

            if ind_full != -1 {
                println!(" -=-=- Hidden notification closed by user, id: {id}");
                notif_q.remove(ind_full as usize);
                display_notifications(&base_frame_p, &notif_q);
            }

            // println!(" !!-=-=-!! Unknown notification closed, id: {id} | reason: {reason}, could not find matching id");
            
            true
        }),
    );

    conn.start_receive(
        mr_notification_delivered,
        Box::new(move |message: Message, _| {
            match message.read1::<u32>() {
                Ok(id) => {
                    let destination = message.destination().unwrap().to_string();

                    let mut pending_notif_q = pending_notification_q.write().unwrap();
                    match pending_notif_q.iter_mut().rev().find(|notif| notif.sender == destination) {
                        Some(notif) => {
                            notif.id = id;
                            println!("Notification delivered, set it id to {id} | reply to {destination}");
                            let settings = &notif.settings;
                            if settings.flash_on_notify {
                                let frame = get_frame_by_condition(&base_frame, &|(index, color)| {
                                    let pos = num2xy(index);
                                    if pos.x >= ROW_LENGTH - 4 { settings.color } else { *color }
                                });
                                flash_frame(&frame, &base_frame, 300, 1000, 300);
                            }
                        },
                        None => {
                            // println!("! Unknown delivery to {destination}, could not find matching sender");
                        },
                    }

                    // cleanup broken notifications
                    let deadline_time = get_timestamp() + notification_delivery_timeout;
                    pending_notif_q.retain(|notif| notif.timestamp <= deadline_time);
                }
                Err(_) => {
                    // println!("Unknown message: {:?}: {e}", message)
                }
            };
            true
        }),
    );

    loop {
        conn.process(Duration::from_millis(1000)).unwrap();
    }
}
