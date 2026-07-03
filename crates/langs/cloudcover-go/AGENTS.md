# cloudcover-go

Provides Go language functionality for CloudCover. `analyzer` contains a Go library which uses `golang.org/x/tools/go` to generate callgraphs for Go source code. The crate static links to that library and and uses it to identify SDK methods used by the source code.
