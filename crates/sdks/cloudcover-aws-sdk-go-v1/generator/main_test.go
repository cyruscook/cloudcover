package main

import (
	"bufio"
	"errors"
	"io"
	"reflect"
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
func serviceMetadata(name string) string {
	return "package service\n\nconst ServiceName = " + strconv.Quote(name) + "\n"
}

func TestParseServiceUsesCanonicalServiceName(t *testing.T) {
	t.Parallel()

	const source = `package cloudwatchlogs

type CloudWatchLogs struct{}

func (c *CloudWatchLogs) CreateLogGroup() {}
func (c *CloudWatchLogs) CreateLogGroupRequest() {}
`
	rows, err := parseService([]byte(source), []byte(serviceMetadata("logs")), "cloudwatchlogs")
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].APIMethods[0] != (apiMethod{Service: "logs", Name: "CreateLogGroup"}) {
		t.Fatalf("parseService() = %#v, want canonical logs service", rows)
	}
}
func serviceState(service, method string) sdkState {
	row := mappingRow{
		Package:    "github.com/aws/aws-sdk-go/service/" + service,
		Receiver:   "Client",
		Method:     method,
		APIMethods: []apiMethod{{Service: service, Name: method}},
	}
	return sdkState{service: map[string]mappingRow{rowKey(row): row}}
}

func mergeStates(states ...sdkState) sdkState {
	merged := sdkState{}
	for _, state := range states {
		for service, rows := range state {
			merged[service] = rows
		}
	}
	return merged
}

func TestUpdateServiceReadsValidBlob(t *testing.T) {
	t.Parallel()

	const service = "widgets"
	const source = `package widgets

type Client struct{}

func (c *Client) ListWidgets() {}
func (c *Client) ListWidgetsRequest() {}
`
	batch := testGitBatch(
		"object blob " + strconv.Itoa(len(source)) + "\n" + source + "\n" +
			"object blob " + strconv.Itoa(len(serviceMetadata("widgets"))) + "\n" + serviceMetadata("widgets") + "\n",
	)
	state := serviceState(service, "OldMethod")

	if _, err := updateService(batch, "v1.2.3", service, state); err != nil {
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

	if _, err := updateService(batch, "v1.2.3", service, state); err != nil {
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
func TestUpdateServiceLeavesStateUntouchedOnParseError(t *testing.T) {
	t.Parallel()

	const source = "package widgets\nfunc (\n"
	state := serviceState("widgets", "OldMethod")
	want := serviceState("widgets", "OldMethod")
	batch := testGitBatch(
		"object blob " + strconv.Itoa(len(source)) + "\n" + source + "\n" +
			"object blob " + strconv.Itoa(len(serviceMetadata("widgets"))) + "\n" + serviceMetadata("widgets") + "\n",
	)

	if _, err := updateService(batch, "v1.2.3", "widgets", state); err == nil {
		t.Fatal("updateService() error = nil, want parse failure")
	}
	if !equalStates(state, want) {
		t.Errorf("updateService() modified state after parse failure: %#v", state)
	}
}

func TestUpdateServiceReturnsGitBatchFailures(t *testing.T) {
	t.Parallel()

	for _, test := range []struct {
		name string

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
			_, err := updateService(testGitBatch(test.response), "v1.2.3", "widgets", state)
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

	_, err := updateService(batch, "v1.2.3", "widgets", state)
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
	for service, wantRows := range want {
		gotRows, ok := got[service]
		if !ok || len(gotRows) != len(wantRows) {
			return false
		}
		for key, wantRow := range wantRows {
			gotRow, ok := gotRows[key]
			if !ok || !equalRow(gotRow, wantRow) {
				return false
			}
		}
	}
	return true
}

func TestUpdateServiceProducesChangedServiceDelta(t *testing.T) {
	t.Parallel()

	const addedSource = `package added

type Client struct{}

func (c *Client) CreateAdded() {}
func (c *Client) CreateAddedRequest() {}
`
	const updatedSource = `package updated

type Client struct{}

func (c *Client) NewMethod() {}
func (c *Client) NewMethodRequest() {}
`
	batch := testGitBatch(
		"object blob " + strconv.Itoa(len(addedSource)) + "\n" + addedSource + "\n" +
			"object blob " + strconv.Itoa(len(serviceMetadata("added"))) + "\n" + serviceMetadata("added") + "\n" +
			"v1.2.3:service/deleted/api.go missing\n" +
			"object blob " + strconv.Itoa(len(updatedSource)) + "\n" + updatedSource + "\n" +
			"object blob " + strconv.Itoa(len(serviceMetadata("updated"))) + "\n" + serviceMetadata("updated") + "\n",
	)
	state := mergeStates(
		serviceState("unchanged", "ListUnchanged"),
		serviceState("deleted", "OldMethod"),
		serviceState("updated", "OldMethod"),
	)
	got := release{ModuleVersion: "1.2.3"}
	for _, service := range []string{"added", "deleted", "updated"} {
		change, err := updateService(batch, "v1.2.3", service, state)
		if err != nil {
			t.Fatalf("updateService(%q): %v", service, err)
		}
		got.Remove = append(got.Remove, change.Remove...)
		got.Upsert = append(got.Upsert, change.Upsert...)
	}
	sortRelease(&got)

	want := release{
		ModuleVersion: "1.2.3",
		Remove: [][3]string{
			{"github.com/aws/aws-sdk-go/service/deleted", "Client", "OldMethod"},
			{"github.com/aws/aws-sdk-go/service/updated", "Client", "OldMethod"},
		},
		Upsert: [][]any{
			{"github.com/aws/aws-sdk-go/service/added", "Client", "CreateAdded", [][]string{{"added", "CreateAdded"}}},
			{"github.com/aws/aws-sdk-go/service/updated", "Client", "NewMethod", [][]string{{"updated", "NewMethod"}}},
		},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("release delta = %#v, want %#v", got, want)
	}

	wantState := mergeStates(
		serviceState("unchanged", "ListUnchanged"),
		serviceState("added", "CreateAdded"),
		serviceState("updated", "NewMethod"),
	)
	if !equalStates(state, wantState) {
		t.Errorf("final state = %#v, want %#v", state, wantState)
	}
}
