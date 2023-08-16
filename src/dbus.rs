use std::{
    collections::HashMap,
    error::Error,
    fs,
    sync::{atomic::Ordering, Arc, RwLock},
    time::Duration,
    vec,
};

use crate::utils::{
    composite, flash_color, get_timestamp, parse_hex, Notification, NotificationSettings,
    ProgressMap, BLACK, BLUE, GREEN, PURPLE, RED, SCREEN_LOCKED, WHITE, PROGRESS_STEP,
};
use dbus::{
    arg::{prop_cast, PropMap},
    blocking::Connection,
    channel::MatchingReceiver,
    message::MatchRule,
    Message,
};
use serde_json::Value;

fn get_full_match_rule<'a>(interface: &'a str, path: &'a str, member: &'a str) -> MatchRule<'a> {
    return MatchRule::with_member(
        MatchRule::with_interface(MatchRule::with_path(MatchRule::new(), path), interface),
        member,
    );
}

pub fn process_dbus() -> Result<(), Box<dyn Error>> {
    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let conn = Connection::new_session()?;

    let pending_notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));
    let notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));

    let mut notification_map = HashMap::new();
    let progress_map = Arc::new(ProgressMap::new());

    let config_j: Value = serde_json::from_str(
        fs::read_to_string("notification_config.json")
            .expect("Error reading notification config")
            .as_str(),
    )?;

    let config_settings = &config_j["notification_map"].as_object().unwrap();
    for (key, value) in config_settings.iter() {
        println!("Loaded {} from notification map", key);
        notification_map.insert(
            key.to_owned(),
            Arc::new(NotificationSettings {
                color: parse_hex(value["color"].as_str().unwrap()),
                flash_on_auto_close: parse_hex(value["flash_on_auto_close"].as_str().unwrap()),
                flash_on_notify: value["flash_on_notify"].as_bool().unwrap(),
                important: value["important"].as_bool().unwrap(),
            }),
        );
    }

    let config_progress = &config_j["progress_map"].as_object().unwrap();
    for (key, value) in config_progress.iter() {
        println!("Loaded {} from progress map", key);
        progress_map.insert(key.to_owned(), (parse_hex(value.as_str().unwrap()), 0.0));
    }

    let notification_delivery_timeout = 2000;

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

    let notification_qc = notification_q.clone();
    let progress_mapc = progress_map.clone();
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

            let progress_delta;
            {
                let mut tuple = progress_mapc
                    .entry(source.to_string())
                    .or_insert((WHITE, 0.0));
                progress_delta = (tuple.1 - progress).abs();

                if progress_delta > PROGRESS_STEP {
                    tuple.1 = progress;
                }

                println!("Notification progress for {source} = {progress}");
            }

            // flash in special cases
            if progress == 0.0 {
                let color = if progress_visible {
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
                flash_color(color, 350, &progress_mapc, &notification_qc);
            } else if progress_delta > PROGRESS_STEP {
                // recomposite if progress changed to not cause stalled animations
                composite(&progress_mapc, &notification_qc, None);
            }

            return true;
        }),
    );

    let screen_locked_p = SCREEN_LOCKED.clone();
    let notification_qc = notification_q.clone();
    let progress_map_qc = progress_map.clone();
    conn.start_receive(
        mr_screen,
        Box::new(move |message: Message, _| {
            let locked: bool = message.read1().unwrap();
            println!("Screen locked/unlocked: {locked}");
            screen_locked_p.store(locked, Ordering::Relaxed);
            composite(&progress_map_qc, &notification_qc, Some(1500));
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

            match notification_map.get(application.as_str()) {
                Some(arc_settings) => {
                    pending_notif_q.push(Notification {
                        id: 0,
                        sender,
                        timestamp: get_timestamp(),
                        settings: arc_settings.clone(),
                    });
                }
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
    let notification_qc = notification_q.clone();
    let progress_map_qc = progress_map.clone();
    conn.start_receive(
        mr_notification_closed,
        Box::new(move |message: Message, _| {
            let (id, reason): (u32, u32) = message.read2().unwrap();

            let mut pending_notif_q = pending_notification_qc.write().unwrap();

            let ind: i64 = find_in_notif_q(id, &pending_notif_q);
            
            if ind != -1 {
                let notif = pending_notif_q.remove(ind as usize);

                if reason == 1 {
                    println!(" -=-=- Pending notification closed automatically, id: {id}");

                    let settings = &notif.settings;

                    if settings.flash_on_auto_close != BLACK {
                        // reason = 1 - expired, 2 - user, 3 - auto, 4 - other
                        flash_color(
                            settings.flash_on_auto_close,
                            500,
                            &progress_map_qc,
                            &notification_qc,
                        );
                    }

                    if settings.important {
                        notification_qc.write().unwrap().push(notif);
                        println!("Moved pending notification {id} to display queue");
                        composite(&progress_map_qc, &notification_qc, Some(200));
                    }

                } else {
                    println!(" -=-=- Pending notification closed by user or automatically, id: {id} | reason: {reason}");
                }
                return true;
            }

            let ind_full: i64 = find_in_notif_q(id, &notification_qc.read().unwrap());

            if ind_full != -1 {
                println!(" -=-=- Hidden notification closed id: {id} | reason: {reason}");
                notification_qc.write().unwrap().remove(ind_full as usize);
                composite(&progress_map_qc, &notification_qc, Some(200));
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
                                flash_color(settings.color, 900, &progress_map, &notification_q);
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
