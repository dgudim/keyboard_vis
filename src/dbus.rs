use std::{
    collections::HashMap,
    error::Error,
    sync::{atomic::Ordering, Arc, RwLock},
    time::Duration,
    vec,
};

use crate::{utils::{
    composite, flash_color, get_timestamp, parse_hex, Notification, NotificationSettings,
    ProgressMap,
}, consts::*, ControllerInfo};
use dbus::{
    arg::{prop_cast, PropMap},
    blocking::Connection,
    channel::MatchingReceiver,
    message::MatchRule,
    Message,
};
use log::{info, warn};
use serde_json::Value;

fn get_full_match_rule<'a>(interface: &'a str, path: &'a str, member: &'a str) -> MatchRule<'a> {
    return MatchRule::with_member(
        MatchRule::with_interface(MatchRule::with_path(MatchRule::new(), path), interface),
        member,
    );
}

pub fn process_dbus(config_j: Value, keyboard_info: ControllerInfo) -> Result<(), Box<dyn Error>> {
    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let conn = Connection::new_session()?;

    let keyboard_info_arc = Arc::new(keyboard_info);

    let pending_notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));
    let notification_q = Arc::new(RwLock::new(Vec::<Notification>::new()));

    let mut notification_map = HashMap::new();
    let progress_map = Arc::new(ProgressMap::new());

    for (key, value) in config_j["notification_map"].as_object().unwrap().into_iter() {
        info!("Loaded {} from notification map", key);
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
    
    for (key, value) in config_j["progress_map"].as_object().unwrap().into_iter() {
        info!("Loaded {} from progress map", key);
        progress_map.insert(key.to_owned(), (parse_hex(value.as_str().unwrap()), 0.0));
    }

    let notification_delivery_timeout = 2000;

    let matchrule_progress = MatchRule::new_signal("com.canonical.Unity.LauncherEntry", "Update");
    let matchrule_screen = get_full_match_rule(
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "ActiveChanged",
    );
    let matchrule_notification_closed = get_full_match_rule(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "NotificationClosed",
    );

    let matchrule_notification_opened = get_full_match_rule(
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

    let matchrule_notification_delivered = MatchRule::with_sender(
        MatchRule::with_type(MatchRule::new(), dbus::MessageType::MethodReturn),
        notification_server_name,
    );

    // become monitor, match all the necessary methods/signals
    dbus_proxy.method_call(
        "org.freedesktop.DBus.Monitoring",
        "BecomeMonitor",
        (
            vec![
                matchrule_progress.match_str(),
                matchrule_screen.match_str(),
                matchrule_notification_closed.match_str(),
                matchrule_notification_opened.match_str(),
                matchrule_notification_delivered.match_str(),
            ],
            0u32,
        ),
    )?;

    conn.start_receive(
        matchrule_progress,
        Box::new({

            let notification_q = notification_q.clone();
            let progress_map = progress_map.clone();
            let keyboard_info_arc = keyboard_info_arc.clone();

            move |message: Message, _| {
                let (source, props): (&str, PropMap) = message.read2().unwrap();
                
                let progress: f64 = *prop_cast(&props, "progress").unwrap_or(&0.0);
                let progress_visible: bool = *prop_cast(&props, "progress-visible").unwrap_or(&true);
                let count: i32 = *prop_cast(&props, "count").unwrap_or(&0);

                let progress_delta;
                {
                    let mut tuple = progress_map
                        .entry(source.to_string())
                        .or_insert((WHITE, 0.0));
                    progress_delta = (tuple.1 - progress).abs();

                    if progress_delta > PROGRESS_STEP {
                        tuple.1 = progress;
                    }

                    info!("Notification progress for {source} = {progress}");
                }

                // flash in special cases
                if progress == 0.0 {
                    let color = if progress_visible {
                        if count > 1 {
                            GREEN // visible notification with visible progress that has (just started probably)
                        } else {
                            RED // invisible (probably closed) notification with visible progress (idk when this occurs, probably not even on notifications)
                        }
                    } else if count > 1 {
                        BLUE // visible notification without visible progress (idk when this occurs)
                    } else {
                        PURPLE // invisible notification without visible progress (spectacle call, download finished)
                    };
                    flash_color(&keyboard_info_arc, color, 350, &progress_map, &notification_q);
                } else if progress_delta > PROGRESS_STEP {
                    // recomposite if progress changed to not cause stalled animations
                    composite(&keyboard_info_arc, &progress_map, &notification_q, None);
                }
                true
            }
        })
    );

    conn.start_receive(
        matchrule_screen,
        Box::new({
            
            // Copy all the necessary stuff to move into closure
            let screen_locked = SCREEN_LOCKED.clone();
            let notifications = notification_q.clone();
            let progress_map = progress_map.clone();
            let keyboard_info_arc = keyboard_info_arc.clone();

            move |message: Message, _| {
                let locked: bool = message.read1().unwrap();
                info!("Screen locked/unlocked: {locked}");
                // Store screen locked state
                screen_locked.store(locked, Ordering::Relaxed);
                // Animate!
                composite(&keyboard_info_arc, &progress_map, &notifications, Some(1500));
                true
            }
        })
    );
    
    conn.start_receive(
        matchrule_notification_opened,
        Box::new({
            
            // Clone Arc for the notification queue
            let pending_notification = pending_notification_q.clone();

            move |message: Message, _| {
                let (application, _, _, summary): (String, u32, String, String) =
                    message.read4().unwrap();
                let sender = message.sender().unwrap().to_string();
                info!("Notification sent from {application} ({sender}) | {summary}");
                let mut pending_notif_q = pending_notification.write().unwrap();

                match notification_map.get(application.as_str()) {
                    Some(arc_settings) => {
                        pending_notif_q.push(Notification {
                            id: 0,
                            sender,
                            timestamp: get_timestamp(),
                            settings: arc_settings.clone(),
                        });
                    }
                    None => warn!("Notification isn't in the map, ignoring"),
                };
                true
            }
        })
    );

    conn.start_receive(
        matchrule_notification_closed,
        Box::new({
            
            let find_in_notif_q = |id: u32, notif_q: &Vec<Notification>| -> Option<usize> {
                return notif_q.iter().position(|notif| notif.id == id);
            };

            let pending_notification_q = pending_notification_q.clone();
            let notification_q = notification_q.clone();
            let progress_map = progress_map.clone();
            let keyboard_info_arc = keyboard_info_arc.clone();

            move |message: Message, _| {
                let (id, reason): (u32, u32) = message.read2().unwrap();

                let mut pending_notif_q = pending_notification_q.write().unwrap();

                let ind: Option<usize> = find_in_notif_q(id, &pending_notif_q);
                
                if let Some(ind) = ind {
                    let notif = pending_notif_q.remove(ind);

                    // https://specifications.freedesktop.org/notification-spec/notification-spec-latest.html
                    // reason = 1 - expired, 2 - user, 3 - auto, 4 - other
                    if reason != 1 {
                        info!(" -=-=- Pending notification closed by user or automatically, id: {id} | reason: {reason}");
                        return true
                    }

                    info!(" -=-=- Pending notification expired and closed, id: {id}");

                    let settings = &notif.settings;

                    if settings.flash_on_auto_close != BLACK {
                        flash_color(&keyboard_info_arc,
                            settings.flash_on_auto_close,
                            500,
                            &progress_map,
                            &notification_q,
                        );
                    }

                    if settings.important {
                        notification_q.write().unwrap().push(notif);
                        info!("Moved pending notification {id} to display queue");
                        composite(&keyboard_info_arc, &progress_map, &notification_q, Some(200));
                    }

                    return true;
                }

                let ind_full: Option<usize> = find_in_notif_q(id, &notification_q.read().unwrap());

                if let Some(ind_full) = ind_full {
                    info!(" -=-=- Hidden notification closed id: {id} | reason: {reason}");
                    notification_q.write().unwrap().remove(ind_full);
                    composite(&keyboard_info_arc, &progress_map, &notification_q, Some(200));
                }

                // warn!(" !!-=-=-!! Unknown notification closed, id: {id} | reason: {reason}, could not find matching id");

                true
            }
        })
    );

    conn.start_receive(
        matchrule_notification_delivered,
        Box::new(move |message: Message, _| {
            match message.read1::<u32>() {
                Ok(id) => {
                    let destination = message.destination().unwrap().to_string();

                    let mut pending_notif_q = pending_notification_q.write().unwrap();
                    match pending_notif_q.iter_mut().rev().find(|notif| notif.sender == destination) {
                        Some(notif) => {
                            notif.id = id;
                            info!("Notification delivered, set its id to {id} | reply to {destination}");
                            let settings = &notif.settings;
                            if settings.flash_on_notify {
                                flash_color(&keyboard_info_arc, settings.color, 900, &progress_map, &notification_q);
                            }
                        },
                        None => {
                            // warn!("! Unknown delivery to {destination}, could not find matching sender");
                        },
                    }

                    // cleanup broken notifications
                    let deadline_time = get_timestamp() + notification_delivery_timeout;
                    pending_notif_q.retain(|notif| notif.timestamp <= deadline_time);
                }
                Err(_) => {
                    // warn!("Unknown message: {:?}: {e}", message)
                }
            };
            true
        }),
    );

    loop {
        conn.process(Duration::from_millis(1000)).unwrap();
        if ABOUT_TO_SHUTDOWN.load(Ordering::Relaxed) > 1 {
            info!("Exit");
            return Ok(());
        }
    }
}
