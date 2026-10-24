# AWS SDK Go v1 compiled mappings

The JSON history and public lookup APIs are unchanged. Binary encoding interns
identical complete row-ID snapshots, including nonadjacent repeats and empty
snapshots. Version entries may share a start/length pair; exact recorded version
membership remains authoritative. Runtime lookup requires no delta replay or
allocation.

On the 1,833-release checked-in history at c5a2294, the generated binary decreases
from 31,106,902 to 2,337,134 bytes (28,769,768 saved). These are generated blob sizes,
not linked executable measurements. `history_equivalence` compares JSON replay to
the public compiled decoder for every release. No JSON schema migration is needed.
