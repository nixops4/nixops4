<!--
  Context:
    - Concepts have already been outlined in ../concepts/resource.md
    - A generated schema reference documentation is available in
      - wrapper: schema/resource-v0.md
      - generated: doc/manual/src/schema/resource-schema-v0.gen.md
  Purpose of this page:
    - Describe the technical details of the interface transport
    - Provide a readable, high level description of the protocol, as the schema page is extremely verbose
    - "Link" the concepts to the schema
-->

# Resource Provider Interface

Note that the resource provider interface is still in development.

The interface between a resource provider and NixOps consist of expectations about:
- The Nix expression that builds the provider and its output
- The process that runs the provider
- A JSON-lines protocol between the provider and NixOps

## Nix Expression

TODO

## Process

NixOps launches the resource provider process built in the previous step.

It communicates with the provider over the standard input and output streams using a JSON-lines protocol.
Standard error is used for logging, and is line-buffered.

TODO: exit behaviors

## Protocol

JSON-lines is a textual protocol where each line is a JSON value that is rendered without line breaks.
Each line must contain a single JSON value, and be terminated with a newline character.

We will refer to the JSON value that takes the line as a _message_.

Messages going from NixOps to the provider are _requests_, and messages going from the provider to NixOps are _responses_.

The content of the messages is specified in the [resource schema](../schema/resource-v0.md).

### Message Flow

Each request-response exchange follows this pattern:

1. NixOps sends a single request message to the provider's stdin
2. The provider processes the request and sends a single response message to stdout

The provider may write diagnostic messages to stderr at any time.

### Request and Response Structure

Both requests and responses use a wrapper object where the key indicates the message type (see [`Request`](../schema/resource-v0.md#1-property-request) and [`Response`](../schema/resource-v0.md#2-property-response)):

```json
// Request
{
  "createResourceRequest": {
    "type": "memo",
    "inputProperties": { "initialize_with": "hello" },
    "isStateful": true
  }
}

// Response
{
  "createResourceResponse": {
    "outputProperties": { "value": "hello" }
  }
}
```

### Operations

The protocol supports the following operations:

**Create**: Provisions a new resource. The request includes:
- `type`: The resource type identifier
- `inputProperties`: Configuration values for the resource
- `isStateful`: Whether state persistence will be provided

See [`CreateResourceRequest`](../schema/resource-v0.md#11-property-createresourcerequest) and [`CreateResourceResponse`](../schema/resource-v0.md#21-property-createresourceresponse) in the schema documentation.

**Update**: Modifies an existing stateful resource. The request includes:
- `resource`: The current resource state (type, input properties, and output properties)
- `inputProperties`: New configuration values

See [`UpdateResourceRequest`](../schema/resource-v0.md#12-property-updateresourcerequest) and [`UpdateResourceResponse`](../schema/resource-v0.md#22-property-updateresourceresponse) in the schema documentation.

### State Operations

Resources that provide state storage implement additional operations:

**State Read**: Retrieves the complete state managed by a state resource. The request includes:
- `resource`: The state resource (type, input properties, and output properties)

The response contains the full state as a JSON object representing all managed resources.

**State Event**: Records a change event to the state. The request includes:
- `resource`: The state resource
- `event`: The operation that produced this change (e.g., "create", "update", "destroy")
- `nixopsVersion`: The version of NixOps that produced this event
- `patch`: JSON Patch operations to apply to the state

These operations use JSON Patch ([RFC 6902](https://tools.ietf.org/html/rfc6902)) to track incremental state changes, enabling efficient state updates and historical tracking.

### State Persistence

The `isStateful` flag in create requests indicates whether the resource will have access to persistent state storage. Resource types that require state must validate this flag and fail if state persistence is not available.

State resources manage the persistence layer for stateful resources, providing operations to read the current state and record state changes as JSON Patch events.
