#!/bin/bash
cd "$(dirname "$0")"

sleep 5
./keyboard_notification_manager > logs.txt
