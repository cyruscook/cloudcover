# CloudCover

A project to store data on the available API methods, associated permissions, and corresponding SDK methods, across different cloud providers.

Users should be able to discover the required permissions for calling given API methods. This can be done through a web interface, or through static analysis of their software to identify SDK methods within the callgraph.

Initially we will only support AWS, however we want to support other cloud providers in the future.

For AWS, we will use the Service Authorization Reference (https://servicereference.us-east-1.amazonaws.com/) as our source of truth. It is incomplete, paticularly on SDK methods (it only provides boto3 data), so we will need to extend it.
