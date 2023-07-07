use std::{collections::HashMap, error::Error, time::Duration};

use dbus::{blocking::Connection, message::MatchRule};

use crate::utils::{fade_into_frame, Frame, BLACK_SUBSTRATE};

pub fn process_dbus(base_frame: Frame) -> Result<(), Box<dyn Error>> {
    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let conn = Connection::new_session()?;

    let mr_progress = MatchRule::new_signal("com.canonical.Unity.LauncherEntry", "Update");
    conn.add_match(mr_progress, |_: (), _, message| {
        let (data1, data2): (&str, HashMap<&str, u16>) =
            message.read2().expect("Error reading data");
        println!("File progress: {} : {:#?}", data1, message.get_items());
        true
    })?;

    let mr_screen = MatchRule::with_member(
        MatchRule::with_interface(
            MatchRule::with_path(MatchRule::new(), "/org/freedesktop/ScreenSaver"),
            "org.freedesktop.ScreenSaver",
        ),
        "ActiveChanged",
    );
    conn.add_match(mr_screen, move |_: (), _, message| {
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
    })?;

    // let mr_screen = MatchRule::new();
    // let signal_screen = conn
    //     .add_match(mr_screen)
    //     .await?
    //     .cb(|message, (_,): (String,)| {
    //         println!("Screen locked/unlocked {:#?}", message);
    //         true
    //     });

    loop {
        conn.process(Duration::from_millis(1000)).unwrap();
    }
}
