package source

import "example.com/cloudcover/dependency"

func init() { dependency.Initialize() }

// Library entrypoints are roots even without a main function.
func Exported() { dependency.Wrapper() }

// All user source is analyzed, including private functions and their closures.
func notCalled() {
	call := func() { dependency.FromClosure() }
	call()
}
