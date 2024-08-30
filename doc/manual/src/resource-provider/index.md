# Resource Provider

A _resource provider_ is the component that is responsible for carrying out the operations that create, update, and delete the real world entity that a resource represents.

This section of the manual focuses on the implementation of resource providers.
It is intended for developers who need to write custom resource providers for NixOps4.
This is not always necessary, as a suitable resource provider may already exist, or in other cases it is possible to build a module that achieves the desired effect using existing resource providers.
