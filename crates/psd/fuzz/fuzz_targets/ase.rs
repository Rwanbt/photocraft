//! Fuzz Adobe .ase palette parsing and normalized write/read fidelity.
//! Truncated or malformed input must not crash the importer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_psd::ase;

fuzz_target!(|data: &[u8]| {
    let Ok(swatches) = ase::parse(data) else {
        return;
    };
    let Ok(serialized) = ase::write(&swatches) else {
        return;
    };
    let reopened = ase::parse(&serialized).expect("writer must emit readable ASE");
    assert_eq!(ase::write(&reopened).expect("second write"), serialized);
});
