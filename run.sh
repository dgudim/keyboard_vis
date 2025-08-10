#!/bin/bash
cd "$(dirname "$0")"

RUST_LOG=INFO ./keyboard_notification_manager >logs.txt
