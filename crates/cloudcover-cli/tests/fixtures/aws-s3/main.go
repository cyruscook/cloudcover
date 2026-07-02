package main

import (
	"context"

	"github.com/aws/aws-sdk-go-v2/service/s3"
)

func main() {
	var client s3.Client
	_, _ = client.GetObject(context.Background(), &s3.GetObjectInput{})
}
