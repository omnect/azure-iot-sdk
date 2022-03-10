# azure-iot-sdk

This repository provides an object oriented wrapper of the rust unsafe bindings for the azure-iot-sdk from [azure-iot-sdk-sys](https://github.com/ICS-DeviceManagement/azure-iot-sdk-sys.git).

A reference implementation showing how this framework might be used can be found [here](https://github.com/ICS-DeviceManagement/iot-client-template-rs).

## Build SDK
If you intend to build the SDK locally the paths to the libraries listed below must be exported.
- export LIB_PATH_AZURESDK=<path to the azure iot sdk c >
- export LIB_PATH_UUID=<path to uid >
- export LIB_PATH_OPENSSL=<path to openssl >
- export LIB_PATH_CURL=<path to curl>

**We intend to provide these dependencies asap.**

## Genrate documetation
The rustdoc documentation of the SDK is not published yet but can be locally created by `cargo doc --lib --no-deps --open`.

# License

Licensed under either of
* Apache License, Version 2.0, (./LICENSE-APACHE or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license (./LICENSE-MIT or <http://opensource.org/licenses/MIT>)
at your option.

# Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

Copyright (c) 2021 conplement AG
