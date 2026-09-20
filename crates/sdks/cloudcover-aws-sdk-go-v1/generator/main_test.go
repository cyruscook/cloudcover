package main

import (
	"bufio"
	"errors"
	"io"
	"strconv"
	"strings"
	"testing"
)

type testWriteCloser struct {
	io.Writer
}

func (testWriteCloser) Close() error { return nil }

type failingWriteCloser struct {
	err error
}

func (writer failingWriteCloser) Write([]byte) (int, error) {
	return 0, writer.err
}

func (failingWriteCloser) Close() error { return nil }

type missingObjectError interface {
	error
	MissingObject()
}

func testGitBatch(response string) *gitBatch {
	return &gitBatch{
		input:  testWriteCloser{Writer: io.Discard},
		output: bufio.NewReader(strings.NewReader(response)),
	}
}

func serviceState(service, method string) sdkState {
	row := mappingRow{
		Package:    "github.com/aws/aws-sdk-go/service/" + service,
		Receiver:   "Client",
		Method:     method,
		APIMethods: []apiMethod{{Service: service, Name: method}},
	}
	return sdkState{rowKey(row): row}
}

func TestUpdateServiceReadsValidBlob(t *testing.T) {
	t.Parallel()

	const service = "widgets"
	const source = `package widgets

type Client struct{}

func (c *Client) ListWidgets() {}
func (c *Client) ListWidgetsRequest() {}
`
	batch := testGitBatch("object blob " + strconv.Itoa(len(source)) + "\n" + source + "\n")
	state := serviceState(service, "OldMethod")

	if err := updateService(batch, "v1.2.3", service, state); err != nil {
		t.Fatal(err)
	}

	want := serviceState(service, "ListWidgets")
	if !equalStates(state, want) {
		t.Errorf("updateService() state = %#v, want %#v", state, want)
	}
}

func TestUpdateServiceTreatsExplicitMissingBlobAsDeletedService(t *testing.T) {
	t.Parallel()

	const service = "widgets"
	state := serviceState(service, "OldMethod")
	batch := testGitBatch("v1.2.3:service/widgets/api.go missing\n")

	if err := updateService(batch, "v1.2.3", service, state); err != nil {
		t.Fatal(err)
	}
	if len(state) != 0 {
		t.Errorf("updateService() retained deleted service mappings: %#v", state)
	}

	_, err := testGitBatch("v1.2.3:service/widgets/api.go missing\n").show("v1.2.3", "service/widgets/api.go")
	var missing missingObjectError
	if !errors.As(err, &missing) {
		t.Errorf("show() error = %v, want a typed missing-object error", err)
	}
}

func TestUpdateServiceReturnsGitBatchFailures(t *testing.T) {
	t.Parallel()

	for _, test := range []struct {
		name     string
		response string
	}{
		{name: "malformed header", response: "not a cat-file header\n"},
		{name: "malformed missing response", response: "object missing extra\n"},
		{name: "non-blob object", response: "object tree 0\n\n"},
		{name: "invalid size", response: "object blob -1\n"},
		{name: "short blob", response: "object blob 4\nabc"},
		{name: "invalid blob terminator", response: "object blob 3\nabcx"},
	} {
		t.Run(test.name, func(t *testing.T) {
			state := serviceState("widgets", "OldMethod")
			err := updateService(testGitBatch(test.response), "v1.2.3", "widgets", state)
			if err == nil {
				t.Fatal("updateService() error = nil, want Git batch failure")
			}
			var missing missingObjectError
			if errors.As(err, &missing) {
				t.Fatalf("updateService() error = %v, incorrectly classified as missing", err)
			}
			if len(state) != 1 {
				t.Errorf("updateService() modified state after failed read: %#v", state)
			}
		})
	}
}

func TestUpdateServiceReturnsGitBatchWriteError(t *testing.T) {
	t.Parallel()

	state := serviceState("widgets", "OldMethod")
	batch := &gitBatch{
		input:  failingWriteCloser{err: io.ErrClosedPipe},
		output: bufio.NewReader(strings.NewReader("")),
	}

	err := updateService(batch, "v1.2.3", "widgets", state)
	if !errors.Is(err, io.ErrClosedPipe) {
		t.Fatalf("updateService() error = %v, want errors.Is(err, io.ErrClosedPipe)", err)
	}
	if len(state) != 1 {
		t.Errorf("updateService() modified state after failed request: %#v", state)
	}
}

func equalStates(got, want sdkState) bool {
	if len(got) != len(want) {
		return false
	}
	for key, wantRow := range want {
		gotRow, ok := got[key]
		if !ok || !equalRow(gotRow, wantRow) {
			return false
		}
	}
	return true
}
