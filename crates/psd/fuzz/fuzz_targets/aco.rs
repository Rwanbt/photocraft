//! Fuzz Photoshop .aco palette parsing and normalized write/read fidelity.
//! A structurally valid palette must be re-writable and stable after normalization.
#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_psd::aco;

fuzz_target!(|data: &[u8]| {
    let Ok(swatches) = aco::parse(data) else {
        return;
    };
    let Ok(serialized) = aco::write(&swatches.swatches) else {
        return;
    };
    let reopened = aco::parse(&serialized).expect("writer must emit readable ACO");
    assert_eq!(aco::write(&reopened.swatches).expect("second write"), serialized);
});
