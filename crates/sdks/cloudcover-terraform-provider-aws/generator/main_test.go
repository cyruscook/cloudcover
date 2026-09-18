package main

import "testing"

func TestSDKKeyForCallable(t *testing.T) {
	t.Parallel()

	const ecrPackage = "github.com/aws/aws-sdk-go-v2/service/ecr"
	tests := []struct {
		name       string
		pkg        string
		receiver   string
		method     string
		expected   sdkMethodKey
		expectedOK bool
	}{
		{
			name:       "client method",
			pkg:        ecrPackage,
			receiver:   "Client",
			method:     "DescribeImages",
			expected:   sdkMethodKey{pkg: ecrPackage, receiver: "Client", method: "DescribeImages"},
			expectedOK: true,
		},
		{
			name:       "paginator constructor",
			pkg:        ecrPackage,
			method:     "NewDescribeImagesPaginator",
			expected:   sdkMethodKey{pkg: ecrPackage, receiver: "Client", method: "DescribeImages"},
			expectedOK: true,
		},
		{
			name:   "non paginator function",
			pkg:    ecrPackage,
			method: "NewFromConfig",
		},
		{
			name:   "nested service package",
			pkg:    ecrPackage + "/types",
			method: "NewDescribeImagesPaginator",
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()
			actual, ok := sdkKeyForCallable(test.pkg, test.receiver, test.method)
			if ok != test.expectedOK {
				t.Fatalf("sdkKeyForCallable() ok = %v, want %v", ok, test.expectedOK)
			}
			if actual != test.expected {
				t.Fatalf("sdkKeyForCallable() = %#v, want %#v", actual, test.expected)
			}
		})
	}
}
