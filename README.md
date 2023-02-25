# azure-iot-sdk
Product page: https://www.omnect.io/home

This repository provides an object oriented wrapper of the rust unsafe bindings for the azure-iot-sdk from [azure-iot-sdk-sys](https://github.com/omnect/azure-iot-sdk-sys.git).

A reference implementation showing how this framework might be used can be found [here](https://github.com/omnect/iot-client-template-rs).

# Build

## Dependencies
Please refer to [azure-iot-sdk-sys](https://github.com/omnect/azure-iot-sdk-sys/blob/main/README.md) documentation in order to provide mandatory libraries needed to build azure-iot-sdk successfully.

An error output similar to the following example indicates that libraries are not set correctly:
```
--- stderr
thread 'main' panicked at 'called `Result::unwrap()` on an `Err` value: `"pkg-config" "--libs" "--cflags" "azure-iotedge-sdk-dev"` did not exit successfully: exit status: 1
error: could not find system library 'azure-iotedge-sdk-dev' required by the 'azure-iot-sdk-sys' crate

--- stderr
Package azure-iotedge-sdk-dev was not found in the pkg-config search path.
Perhaps you should add the directory containing `azure-iotedge-sdk-dev.pc'
to the PKG_CONFIG_PATH environment variable
No package 'azure-iotedge-sdk-dev' found
```

## Configuration

In order to create the purposed iot client, the crate must be configured via cargo feature to one of the following types:
- `device_client`
- `module_client`
- `edge_client`

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

---

copyright (c) 2023 conplement AG<br>
Content published under the Apache License Version 2.0 or MIT license, are marked as such. They may be used in accordance with the stated license conditions.
