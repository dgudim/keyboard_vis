mod utils;
mod dbus;
use crate::utils::*;
use crate::dbus::*;

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
            Ok(_) => {},
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

    let center_x = (ROW_LENGTH / 2) as i64;
    let center_y = (ROWS / 2) as i64;
    let corner_dist = ((center_x * center_x + center_y * center_y) as f64).sqrt();
    for target_dist in 0..center_x * 3 {
        let target_dist_f = target_dist as f64;

        enq_frame(
            &GRAY_SUBSTRATE
                .iter()
                .enumerate()
                .map(|(index, gray)| {
                    let pos = num2xy(index as u32);
                    let distance_from_center = (((pos.x as i64 - center_x).pow(2)
                        + (pos.y as i64 - center_y).pow(2))
                        as f64)
                        .sqrt();

                    if distance_from_center < target_dist_f {
                        let i_offset: f64 = target_dist_f - 10.0;
                        let distance_factor = (distance_from_center - i_offset) / corner_dist;

                        return lerp_color(&target_substrate[index], gray, distance_factor);
                    } else if distance_from_center < target_dist_f + 1.0 {
                        // edge lighting
                        return WHITE;
                    }
                    // the rest of the key board
                    return BLACK;
                })
                .collect(),
        );
    }
    
    process_dbus().await?;

    Ok(())
}

fn enq_frame(frame: &Frame) -> () {
    let mut last_frame = LAST_FRAME
        .write()
        .expect("Could not lock mutex to write frame");
    *last_frame = frame.clone();
    match FRAME_Q.push(frame.clone()) {
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
