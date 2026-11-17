import { DynamoDBClient, DescribeTableCommand } from "@aws-sdk/client-dynamodb";
import { DynamoDBDocument, DynamoDBDocumentClient, GetCommand } from "@aws-sdk/lib-dynamodb";

const client = new DynamoDBClient({});
const documentClient = DynamoDBDocumentClient.from(client);
// Factory and lifecycle helpers make no API calls.
DynamoDBDocument.from(client).destroy();

export async function readToken() {
  const response = await documentClient.send(new GetCommand({
    TableName: "sessions",
    Key: { id: "example" },
  }));
  const optional = response.Item?.access_token;
  if (!response.Item) return optional;
  return response.Item.access_token;
}

export async function describe() {
  const response = await client.send(new DescribeTableCommand({ TableName: "sessions" }));
  return response.Table?.TableStatus;
}
