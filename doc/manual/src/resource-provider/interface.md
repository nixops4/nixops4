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

The content of the messages is specified in [`resource-provider-schema.json`](https://github.com/nixops4/nixops4/blob/main/rust/nixops4-resources/resource-provider-schema.json).

<!-- TODO: describe handshake -->

<!-- TODO: describe how the message types relate -->
