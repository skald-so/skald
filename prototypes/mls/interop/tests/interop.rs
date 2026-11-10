//! The interop scenarios in both directions: a device on one library takes each step, devices on
//! the other process it.

use mls_proto_mls_rs::MlsRsDevice;
use mls_proto_openmls::OpenMlsDevice;

mod openmls_to_mls_rs {
    use super::*;

    mls_scenario::interop_tests!(OpenMlsDevice, MlsRsDevice);
}

mod mls_rs_to_openmls {
    use super::*;

    mls_scenario::interop_tests!(MlsRsDevice, OpenMlsDevice);
}
