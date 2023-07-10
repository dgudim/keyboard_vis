mod dbus;
mod utils;
use crate::dbus::*;
use crate::utils::*;

use openrgb::data::{Controller, LED};
use openrgb::OpenRGB;
use std::error::Error;
use std::time::Duration;
use tokio::{net::TcpStream, time::sleep};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // connect to default server at localhost
    let client = OpenRGB::connect().await?;

    let controllers = client.get_controller_count().await?;
    let mut target_controller: Option<Controller> = Option::None;
    let mut target_controller_id: Option<u32> = Option::None;

    // query and print each controller data
    for controller_id in 0..controllers {
        let controller = client.get_controller(controller_id).await?;
        println!("controller {}: {}", controller_id, controller.name);
        if controller.name.eq(KEYBOARD_NAME) {
            target_controller = Option::Some(controller);
            target_controller_id = Some(controller_id);
            break;
        }
    }

    if target_controller == Option::None {
        return Err(format!("{} not found!", KEYBOARD_NAME))?;
    }

    tokio::spawn(async move {
        match render_frames(target_controller_id.unwrap(), &client).await {
            Ok(_) => {}
            Err(e) => {
                print!("Ann error occured in the frame rendering loop: {}", e);
            }
        };
    });

    let target_substrate = get_frame_by_key_names(
        &target_controller.unwrap().leds,
        Vec::from([
            KeyMap {
                keys: Vec::from(["Key: Number Pad", "Key: Num Lock"]),
                color: *NUM_PAD_COLOR,
            },
            KeyMap {
                keys: Vec::from(["Insert", "Delete", "Page", "Arrow", "End", "Home"]),
                color: *FUNCTION_COLOR,
            },
            KeyMap {
                keys: Vec::from(["Print", "Scroll", "Pause"]),
                color: *FUNCTION_COLOR2,
            },
        ]),
        &|_: &LED, index: usize| match index <= 14 {
            true => *TOP_ROW_COLOR,
            false => *MAIN_COLOR,
        },
    );
    
    for target_dist in 0..CENTER_X * 3 {
        let target_dist_f = target_dist as f64;

        let intermediate: Frame = GRAY_SUBSTRATE
            .iter()
            .enumerate()
            .map(|(index, gray)| {
                let pos = num2xy(index);
                let distance_from_center =
                    (((pos.x as i64 - CENTER_X).pow(2) + (pos.y as i64 - CENTER_Y).pow(2)) as f64)
                        .sqrt();

                // center color to gray
                if distance_from_center < target_dist_f {
                    let distance_factor = (distance_from_center - target_dist_f + 7.0) / 4.0; // 7 led offset from the center, 4 led width (offset from the edge)
                    return lerp_color(&target_substrate[index], gray, distance_factor);
                }

                let distance_factor = (distance_from_center - target_dist_f) / 2.0;
                return lerp_color(&WHITE, &BLACK, distance_factor);
            })
            .collect();

        fade_into_frame(&intermediate, FRAME_DELTA * 2) // stretch each frame 2 times
    }

    process_dbus(target_substrate)?;

    Ok(())
}

fn enq_frame(frame: Frame) -> () {
    let mut last_frame = LAST_FRAME
        .write()
        .expect("Could not lock mutex to write frame");
    *last_frame = frame.clone();
    match FRAME_Q.push(frame) {
        Ok(_) => {}
        Err(e) => {
            println!("Error adding frame! ({})", e);
        }
    }
}

async fn render_frames(id: u32, client: &OpenRGB<TcpStream>) -> Result<(), Box<dyn Error>> {
    let frame_delay = Duration::from_millis(FRAME_DELTA as u64);
    loop {
        match FRAME_Q.pop() {
            Ok(frame) => {
                client.update_leds(id, frame).await?;
            }
            Err(_) => {}
        };

        sleep(frame_delay).await;
    }
}
