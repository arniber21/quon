#![no_main]

use libfuzzer_sys::fuzz_target;
use zx::GateRef;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let name_len = (data[0] as usize) % 16;
    if data.len() < 1 + name_len + 1 {
        return;
    }
    if let Ok(name) = std::str::from_utf8(&data[1..1 + name_len]) {
        if name.is_empty() {
            return;
        }
        let qubit = (data[1 + name_len] as usize) % 64;
        let g = GateRef::new(name, vec![qubit]);
        assert_eq!(g.qubits.len(), 1);
        let _ = GateRef::rotation(name, qubit, 0.5);
    }
});
