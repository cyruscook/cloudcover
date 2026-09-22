package main

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func writeSigningNameSource(t *testing.T, serviceDir, name string) {
	t.Helper()
	source := `package example

func serviceAuthOptions() {
	var props any
	smithyhttp.SetSigV4SigningName(&props, "SERVICE")
}
`
	source = strings.Replace(source, "SERVICE", name, 1)
	if err := os.WriteFile(filepath.Join(serviceDir, "auth.go"), []byte(source), 0o600); err != nil {
		t.Fatal(err)
	}
}
func TestLoadFastServiceRowsUsesCanonicalSigningName(t *testing.T) {
	t.Parallel()

	serviceDir := t.TempDir()
	const source = `package cloudwatchlogs

type Client struct{}

func (c *Client) CreateLogGroup() {
	c.invokeOperation(nil, "CreateLogGroup", nil)
}
`
	if err := os.WriteFile(filepath.Join(serviceDir, "api.go"), []byte(source), 0o600); err != nil {
		t.Fatal(err)
	}
	writeSigningNameSource(t, serviceDir, "logs")

	rows, err := loadFastServiceRows(serviceDir, "github.com/aws/aws-sdk-go-v2/service/cloudwatchlogs")
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].APIMethods[0] != (apiMethod{Service: "logs", Name: "CreateLogGroup"}) {
		t.Fatalf("loadFastServiceRows() = %#v, want canonical logs service", rows)
	}
}

func TestLoadPaginatorRowsUsesReceiverClientField(t *testing.T) {
	t.Parallel()

	serviceDir := t.TempDir()
	const source = `package example

import "context"

type Client struct{}
type ListThingsInput struct{}
type ListThingsOutput struct{}
type ListThingsAPIClient interface {
	ListThings(context.Context, *ListThingsInput) (*ListThingsOutput, error)
}

type ListThingsPaginator struct {
	client ListThingsAPIClient
}

type helperClient struct{}

var helper helperClient

func (c *Client) ListThings(ctx context.Context, params *ListThingsInput) (*ListThingsOutput, error) {
	return c.invokeOperation(ctx, "ListThings", params)
}

func (c *Client) GetThings(ctx context.Context, params *ListThingsInput) (*ListThingsOutput, error) {
	return c.invokeOperation(ctx, "GetThings", params)
}

func (c *Client) invokeOperation(ctx context.Context, operation string, params any) (*ListThingsOutput, error) {
	return nil, nil
}

func NewListThingsPaginator(client *Client, params *ListThingsInput) *ListThingsPaginator {
	return &ListThingsPaginator{client: client}
}

func (p *ListThingsPaginator) NextPage(ctx context.Context) (*ListThingsOutput, error) {
	result, err := p.client.ListThings(ctx, &ListThingsInput{})
	helper.GetThings(ctx)
	return result, err
}

func NewFromConfig(config any) *Client {
	return &Client{}
}

func (helperClient) GetThings(context.Context) {}
`
	if err := os.WriteFile(filepath.Join(serviceDir, "api.go"), []byte(source), 0o600); err != nil {
		t.Fatal(err)
	}
	writeSigningNameSource(t, serviceDir, "example")

	const modulePath = "github.com/aws/aws-sdk-go-v2/service/example"
	clientRows, err := loadFastServiceRows(serviceDir, modulePath)
	if err != nil {
		t.Fatal(err)
	}
	rows, err := loadPaginatorRows(serviceDir, modulePath, clientRows)
	if err != nil {
		t.Fatal(err)
	}

	want := map[string]mappingRow{
		"ListThingsPaginator.NextPage": {
			Package:  modulePath,
			Receiver: "ListThingsPaginator",
			Method:   "NextPage",
			APIMethods: []apiMethod{{
				Service: "example",
				Name:    "ListThings",
			}},
		},
	}
	if len(rows) != len(want) {
		t.Fatalf("loadPaginatorRows() returned %d rows, want %d: %#v", len(rows), len(want), rows)
	}
	for _, row := range rows {
		key := row.Method
		if row.Receiver != "" {
			key = row.Receiver + "." + key
		}
		got, ok := want[key]
		if !ok {
			t.Fatalf("unexpected row %#v", row)
		}
		if row.Package != got.Package || row.Receiver != got.Receiver || row.Method != got.Method ||
			len(row.APIMethods) != 1 || row.APIMethods[0] != got.APIMethods[0] {
			t.Errorf("row %#v, want %#v", row, got)
		}
	}
}

func TestLoadFastServiceRowsExtractsOperationFromKnownArgument(t *testing.T) {
	t.Parallel()

	serviceDir := t.TempDir()
	const source = `package example

import "context"

type Client struct{}
type helperClient struct{}

var helper helperClient

func (c *Client) GetThing(ctx context.Context, params any) (any, error) {
	helper.invokeOperation(ctx, "not-an-operation", params)
	return c.invokeOperation(ctx, "GetThing", params)
}

func (c Client) GetValueThing(ctx context.Context, params any) (any, error) {
	return c.invokeOperation(ctx, "GetValueThing", params)
}
`
	if err := os.WriteFile(filepath.Join(serviceDir, "api.go"), []byte(source), 0o600); err != nil {
		t.Fatal(err)
	}
	writeSigningNameSource(t, serviceDir, "example")

	rows, err := loadFastServiceRows(serviceDir, "github.com/aws/aws-sdk-go-v2/service/example")
	if err != nil {
		t.Fatal(err)
	}
	want := []mappingRow{
		{
			Package:  "github.com/aws/aws-sdk-go-v2/service/example",
			Receiver: "Client",
			Method:   "GetThing",
			APIMethods: []apiMethod{{
				Service: "example",
				Name:    "GetThing",
			}},
		},
		{
			Package:  "github.com/aws/aws-sdk-go-v2/service/example",
			Receiver: "Client",
			Method:   "GetValueThing",
			APIMethods: []apiMethod{{
				Service: "example",
				Name:    "GetValueThing",
			}},
		},
	}
	if !slices.EqualFunc(rows, want, func(got, want mappingRow) bool {
		return got.Package == want.Package &&
			got.Receiver == want.Receiver &&
			got.Method == want.Method &&
			slices.Equal(got.APIMethods, want.APIMethods)
	}) {
		t.Errorf("loadFastServiceRows() = %#v, want %#v", rows, want)
	}
}

func TestLoadFastServiceRowsSkipsHelperInvokeOperation(t *testing.T) {
	t.Parallel()

	serviceDir := t.TempDir()
	const source = `package example

import "context"

type Client struct{}
type helperClient struct{}

var helper helperClient

func (c Client) Helper(ctx context.Context, params any) {
	helper.invokeOperation(ctx, "not-an-operation", params)
}
`
	if err := os.WriteFile(filepath.Join(serviceDir, "api.go"), []byte(source), 0o600); err != nil {
		t.Fatal(err)
	}
	writeSigningNameSource(t, serviceDir, "example")
	nestedDir := filepath.Join(serviceDir, "internal")
	if err := os.Mkdir(nestedDir, 0o700); err != nil {
		t.Fatal(err)
	}
	const nestedSource = `package internal

type Client struct{}

func (c *Client) Nested() {
	c.invokeOperation(nil, "not-an-operation", nil)
}
`
	if err := os.WriteFile(filepath.Join(nestedDir, "helper.go"), []byte(nestedSource), 0o600); err != nil {
		t.Fatal(err)
	}

	rows, err := loadFastServiceRows(serviceDir, "github.com/aws/aws-sdk-go-v2/service/example")
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 0 {
		t.Errorf("loadFastServiceRows() = %#v, want no rows", rows)
	}
}

func TestLoadFastServiceRowsRejectsMalformedOperationCall(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name   string
		source string
	}{
		{
			name: "missing operation argument",
			source: `package example

import "context"

type Client struct{}

func (c *Client) Broken(ctx context.Context, params any) (any, error) {
	return c.invokeOperation(ctx, params)
}
`,
		},
		{
			name: "operation argument at wrong position",
			source: `package example

import "context"

type Client struct{}

func (c *Client) Broken(ctx context.Context, params any) (any, error) {
	return c.invokeOperation(ctx, params, "not-an-operation")
}
`,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			serviceDir := t.TempDir()
			if err := os.WriteFile(filepath.Join(serviceDir, "api.go"), []byte(test.source), 0o600); err != nil {
				t.Fatal(err)
			}
			writeSigningNameSource(t, serviceDir, "example")

			_, err := loadFastServiceRows(serviceDir, "github.com/aws/aws-sdk-go-v2/service/example")
			if err == nil {
				t.Fatal("loadFastServiceRows() returned nil error")
			}
			if !strings.Contains(err.Error(), "Client.Broken") || !strings.Contains(err.Error(), "operation") {
				t.Errorf("loadFastServiceRows() error = %q, want Client.Broken operation error", err)
			}
		})
	}
}
