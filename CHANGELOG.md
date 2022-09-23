# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.5] Q3 2022
 - enabled devicestream c2d handling

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
