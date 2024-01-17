# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.7] Q1 2024
 - added environment variable to optionally set do_work frequency 

## [0.11.6] Q4 2023
 - updated eis-utils to 0.3.2

## [0.11.5] Q4 2023
 - improved direct method logs
 - unified string formatting

## [0.11.4] Q4 2023
 - replaced dependency url-encoded with url crate
 - fixed cargo clippy findings

## [0.11.3] Q3 2023
 - fixed cargo clippy findings

## [0.11.2] Q3 2023
 - removed cargo ignore for RUSTSEC-2020-0071

## [0.11.1] Q3 2023
 - fixed encoding for message properties.
 - fixed lint warnings
 - fixed code style

## [0.11.0] Q3 2023
 - fixed potential deadlocks when e.g. direct_method and reported property confirmation
   callbacks share the same thread at the same time in azure-c-sdk
 - a JoinSet now holds tasks that are waiting for confirmation with a timeout

## [0.10.1] Q3 2023
 - fixed inaccuracies in IoTMessage docstrings

## [0.10.0] Q3 2023
 - replaced EventHandler by async API's
 - switched to azure-iot-sdk-c convenient layer which brings its own thread safe runtime
 - bumped to azure-iot-sdk-sys 0.6.0 which supports azure-iot-sdk-c convenient layer
 - bumped to eis-utils 0.3.0 providing async 'request_connection_string_from_eis_with_expiry'

## [0.9.5] Q2 2023
 - removed get_ prefix from getters in order to be conform with rust convention
 - changed omnect git dependencies from ssh to https url's

## [0.9.4] Q2 2023
 - added Copy and Clone trait to enums for better usability in library clients

## [0.9.3] Q1 2023
 - fixed cargo clippy findings

## [0.9.2] Q1 2023
 - fixed cargo clippy findings

## [0.9.1] Q1 2023
 - added PartialEq trait to support better testability
 - prepared readme for open sourcing repository

## [0.9.0] Q1 2023
 - switched to anyhow based errors
 - bumped to azure-iot-sdk-sys 0.5.7

## [0.8.8] Q4 2022
 - bumped to eis-utils 0.2.5
 - ignored RUSTSEC-2020-0071 introduced in eis-utils 0.2.4

## [0.8.7] Q4 2022
 - bumped to eis-utils 0.2.4

## [0.8.6] Q4 2022
 - bumped to eis-utils 0.2.3

## [0.8.5] Q4 2022
 - renamed from ICS-DeviceManagement to omnect github orga

## [0.8.4] Q3 2022
 - add get_sdk_version_string() API
 - bump to azure-iot-sdk-sys 0.5.2

## [0.8.3] Q3 2022
 - bump to azure-iot-sdk-sys 0.5.1

## [0.8.2] Q3 2022
 - bump to azure-iot-sdk-sys 0.5.0

## [0.8.1] Q2 2022
 - bump to azure-iot-sdk-sys 0.4.0

## [0.8.0] Q2 2022
 - added iot edge module support:
   - bumb to azure-iot-sdk-sys 0.3.0
   - introduced cargo features to select client type (device, module, edge)
   - conditional compilation depending on client type

## [0.7.0] Q2 2022
 - removed all avoidable unwraps()
 - switch to impl Into<String> instead of &str or String in public functions

## [0.6.0] Q1 2022
 - added error message for direct_method calls

## [0.5.5] Q1 2022
 - bumped to azure-iot-sdk-sys 0.2.3
 - added link in documentation in order to build azure-iot-sdk-sys

## [0.5.4] Q1 2022
 - bumped to eis-utils 0.2.2

## [0.5.3] Q1 2022
 - added Cargo.audit.ignore
 - bumped to azure-iot-sdk-sys 0.2.2

## [0.5.2] Q1 2022
 - added rustdoc documentation
 - some refactoring

## [0.5.1] Q1 2022
 - updated eis_utils to 0.2.1
 - some code cosmetics

## [0.5.0] Q1 2022
 - refactoring due to support device twins besides module twins

## [0.4.1] Q1 2022
 - handle connection status

## [0.4.0] Q1 2022
 - add D2C messages

## [0.3.0] Q1 2022
 - introduced object oriented interface
 - renamed package and repo to azure-iot-sdk
 - updated dependency to azure-iot-sdk-sys

## [0.2.0] Q4 2021
 - add c_connection_callback() to log module connection and disconnection state

## [0.1.1] Q4 2021
 - fix compiling for 32bit systems

## [0.1.0] Q4 2021
 - initial version
