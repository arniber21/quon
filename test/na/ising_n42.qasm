// RAP Table I regression fixture — OpenQASM 2 ingestion counterpart of
// `test/na/ising_n42.qn` (issue #304, NA-scoped slice of #197).
//
// Structural twin of the `.qn` fixture: two Trotter-shaped steps of a
// 42-qubit nearest-neighbor chain. Each step's "ZZ layer" is split into two
// sequential matchings — even bonds (0,1)(2,3)...(40,41), then odd bonds
// (1,2)(3,4)...(39,40) — so `quon_na`'s ASAP dependency-DAG scheduler (per-
// qubit last-use tracking, identical to the `.qn` extraction path) places
// every gate in a matching on the SAME dag_layer and serializes between
// matchings. Result: layers 0/1 (step 1 even/odd) then 2/3 (step 2
// even/odd) — 4 rydberg stages, 21+20+21+20 = 82 two-qubit gates.
//
// Uses native `cz` (not `cx`/`rzz`): every NA target's `native_gates` lists
// `cz`, and the NA entangling scheduler models each ≥2-qubit gate as one
// symmetric Entangle2 (undirected interaction edge), so `cz` keeps the
// 82/4 pre-flight counts intact — a `cx` would be an equally valid single
// Entangle2 here, but `cz` matches the `.qn` fixture verbatim. Do not
// "simplify" this file into one ascending loop over `range(41)` (that
// chains every gate through its shared neighbor and yields 41 serial layers
// per step) — see `test/na/ising_n42.qn`'s header comment for the rationale.
//
// Acceptance: `quonc test/na/ising_n42.qasm --target
// targets/neutral_atom/rap_table_i.json --emit-resource-report -` reports
// `entangle2_count == 82` and `rydberg_stages == 4` — the same pre-flight
// invariants as the `.qn` fixture.
OPENQASM 2.0;
include "qelib1.inc";
qreg q[42];
// Step 1 — even bonds.
cz q[0],q[1]; cz q[2],q[3]; cz q[4],q[5]; cz q[6],q[7]; cz q[8],q[9];
cz q[10],q[11]; cz q[12],q[13]; cz q[14],q[15]; cz q[16],q[17]; cz q[18],q[19];
cz q[20],q[21]; cz q[22],q[23]; cz q[24],q[25]; cz q[26],q[27]; cz q[28],q[29];
cz q[30],q[31]; cz q[32],q[33]; cz q[34],q[35]; cz q[36],q[37]; cz q[38],q[39];
cz q[40],q[41];
// Step 1 — odd bonds.
cz q[1],q[2]; cz q[3],q[4]; cz q[5],q[6]; cz q[7],q[8]; cz q[9],q[10];
cz q[11],q[12]; cz q[13],q[14]; cz q[15],q[16]; cz q[17],q[18]; cz q[19],q[20];
cz q[21],q[22]; cz q[23],q[24]; cz q[25],q[26]; cz q[27],q[28]; cz q[29],q[30];
cz q[31],q[32]; cz q[33],q[34]; cz q[35],q[36]; cz q[37],q[38]; cz q[39],q[40];
// Step 2 — even bonds.
cz q[0],q[1]; cz q[2],q[3]; cz q[4],q[5]; cz q[6],q[7]; cz q[8],q[9];
cz q[10],q[11]; cz q[12],q[13]; cz q[14],q[15]; cz q[16],q[17]; cz q[18],q[19];
cz q[20],q[21]; cz q[22],q[23]; cz q[24],q[25]; cz q[26],q[27]; cz q[28],q[29];
cz q[30],q[31]; cz q[32],q[33]; cz q[34],q[35]; cz q[36],q[37]; cz q[38],q[39];
cz q[40],q[41];
// Step 2 — odd bonds.
cz q[1],q[2]; cz q[3],q[4]; cz q[5],q[6]; cz q[7],q[8]; cz q[9],q[10];
cz q[11],q[12]; cz q[13],q[14]; cz q[15],q[16]; cz q[17],q[18]; cz q[19],q[20];
cz q[21],q[22]; cz q[23],q[24]; cz q[25],q[26]; cz q[27],q[28]; cz q[29],q[30];
cz q[31],q[32]; cz q[33],q[34]; cz q[35],q[36]; cz q[37],q[38]; cz q[39],q[40];
