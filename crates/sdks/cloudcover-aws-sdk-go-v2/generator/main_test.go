package main

import (
	"os"
	"path/filepath"
	"testing"
)

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
