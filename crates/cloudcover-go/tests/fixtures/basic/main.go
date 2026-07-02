package main

type Client struct{}

func (c *Client) Do() {}

func helper() {}

func main() {
	var client Client
	client.Do()
	helper()
}
