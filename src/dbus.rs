use std::{collections::HashMap, error::Error, time::Duration};

use dbus::{blocking::Connection, channel::MatchingReceiver, message::MatchRule, Message};

use crate::utils::{fade_into_frame, Frame, BLACK_SUBSTRATE};

fn get_full_match_rule<'a>(interface: &'a str, path: &'a str, member: &'a str) -> MatchRule<'a> {
    return MatchRule::with_member(
        MatchRule::with_interface(MatchRule::with_path(MatchRule::new(), path), interface),
        member,
    );
}

pub fn process_dbus(base_frame: Frame) -> Result<(), Box<dyn Error>> {
    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let conn = Connection::new_session()?;

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
    let proxy = conn.with_proxy(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        Duration::from_millis(5000),
    );

    // become monitor, match all the necessary methods/signals
    proxy.method_call(
        "org.freedesktop.DBus.Monitoring",
        "BecomeMonitor",
        (
            vec![
                mr_progress.match_str(),
                mr_screen.match_str(),
                mr_notification_closed.match_str(),
                mr_notification_opened.match_str(),
            ],
            0u32,
        ),
    )?;

    conn.start_receive(
        mr_progress,
        Box::new(|message: Message, _| {
            let (data1, data2): (&str, HashMap<&str, u16>) =
                message.read2().expect("Error reading data");
            println!("File progress: {} : {:#?}", data1, message.get_items());
            true
        }),
    );

    conn.start_receive(
        mr_screen,
        Box::new(move |message: Message, _| {
            let locked: bool = message.read1().expect("Error reading data");
            println!("Screen locked/unlocked {locked}");
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

    conn.start_receive(
        mr_notification_opened,
        Box::new(|message: Message, _| {
            println!("Notification opened {:#?}", message);
            true
        }),
    );

    conn.start_receive(
        mr_notification_closed,
        Box::new(|message: Message, _| {
            println!("Notification closed {:#?}", message);
            true
        }),
    );

    loop {
        conn.process(Duration::from_millis(1000)).unwrap();
    }
}
