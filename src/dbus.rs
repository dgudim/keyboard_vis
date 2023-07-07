use std::collections::HashMap;

use dbus::{channel::MatchingReceiver, message::MatchRule, blocking::Connection};
use dbus_tokio::connection;

pub async fn process_dbus() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the D-Bus session bus (this is blocking, unfortunately).
    let (resource, conn) = connection::new_session_sync()?;

    // The resource is a task that should be spawned onto a tokio compatible
    // reactor ASAP. If the resource ever finishes, you lost connection to D-Bus.
    //
    // To shut down the connection, both call _handle.abort() and drop the connection.
    let handle = tokio::spawn(async {
        let err = resource.await;
        panic!("Lost connection to D-Bus: {}", err);
    });


    let mr_progress = MatchRule::new_signal("com.canonical.Unity.LauncherEntry", "Update");
    let signal_progress = conn
        .add_match(mr_progress)
        .await?
        .cb(|message, (_,): (String,)| {
            let (data1, data2): (&str, HashMap<&str, u16>) =
                message.read2().expect("Error reading data");
            println!("File progress: {} : {:#?}", data1, message.get_items());
            true
        });

    // let mr_screen = MatchRule::new();
    // let signal_screen = conn
    //     .add_match(mr_screen)
    //     .await?
    //     .cb(|message, (_,): (String,)| {
    //         println!("Screen locked/unlocked {:#?}", message);
    //         true
    //     });

    handle.await?;

    // Needed here to ensure the "incoming_signal" object is not dropped too early
    conn.remove_match(signal_progress.token()).await?;
    //conn.remove_match(signal_screen.token()).await?;

    unreachable!()
}
