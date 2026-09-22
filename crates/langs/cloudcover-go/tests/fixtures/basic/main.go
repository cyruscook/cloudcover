package main

type Client struct{}

func (c *Client) Do() {}

type ClientAPI interface {
	Do()
}

func useClient(client ClientAPI) {
	client.Do()
}

func helper() {}

func main() {
	useClient(&Client{})
	helper()
}
