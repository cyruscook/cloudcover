# Historical SDK release rewrites

Go v2 history updates replay the original sequence independently from the output
sequence. New analyses supply selected snapshots; retained releases take their
snapshot from original history. Deltas are then computed against the rewritten
predecessor. Existing successors therefore retain their exact mapping even when
an older inserted snapshot adds, removes or changes operations.

Terraform uses the same two-state model. A replacement now replaces its entire
supported/unsupported status, including clearing a former unsupported reason.
The Go v2 generator regression replays all 469 checked-in service histories and
requires identical release deltas when there are no replacements. Synthetic tests
cover silent successor contamination and conflicting repeated upserts.

No JSON migration or SDK API change is required. CI now executes Rust generator
feature tests and the Go v2 analyzer suite, rather than only compiling generators.
