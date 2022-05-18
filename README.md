# azure-iot-sdk

This repository provides an object oriented wrapper of the rust unsafe bindings for the azure-iot-sdk from [azure-iot-sdk-sys](https://github.com/ICS-DeviceManagement/azure-iot-sdk-sys.git).

A reference implementation showing how this framework might be used can be found [here](https://github.com/ICS-DeviceManagement/iot-client-template-rs).

# Build

Please refer to [azure-iot-sdk-sys](https://github.com/ICS-DeviceManagement/azure-iot-sdk-sys/blob/main/README.md) documentation in order to provide mandatory libraries needed to build azure-iot-sdk successfully.

An error output similar to the following example indicates that libraries are not set correctly:
>error: failed to run custom build command for `azure-iot-sdk-sys v0.3.0 (ssh://git@github.com/ICS-DeviceManagement/azure-iot-sdk-sys.git?tag=0.2.2#0357acbf)`
>
>Caused by:
>  process didn't exit successfully: `/home/osboxes/projects/azure-iot-sdk/target/debug/build/azure-iot-sdk-sys-35a448ef75c7b5ee/build-script-build` (exit status: 101)
>  --- stderr
>  thread 'main' panicked at 'env LIB_PATH_AZURESDK is not available: NotPresent', /home/osboxes/.cargo/git/checkouts/azure-iot-sdk-sys-13093a02cfa1dea4/0357acb/build.rs:11:30

# Generate documentation

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
