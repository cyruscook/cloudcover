# cloudcover-go

This crate analyzes user Go source code and returns discovered Go method references for CloudCover.

Structure:
- `analyzer/`: Go shared library using `golang.org/x/tools/go/packages`, SSA, and static callgraph analysis.
- Rust crate code loads the analyzer output and converts it into `cloudcover-core` method-reference types.
