# WIT

## Introducing an Unstable Interface

It is not possible to make an unstable interface which only has unstable members.
To achieve the same effect, put the new interface behind a `since` gate and make all members `unstable`.

## Stabilizing Members of an Interface

When stabilizing previously unstable members of an interface, the `since` gate on the interface does not need to be updated.
However, all previously `unstable` gated members need to be gated with the new version of the WIT world, and the minor version of the WIT world version needs to be incremented.

## Modifying a Gated Interface

If you are doing non-breaking changes to a gated interface, e.g. changing an unstable record or adding a new function, the since gate on the interface does not need to be incremented.

## Adding new Parameters to a Struct

Adding a new parameter to a struct, even if it optional, is a breaking change from the ABI perspective at the moment.
So it requires either introducing a new struct and methods associated with it, or a new major version of the wit world.
There is ideas for a more looser sub-typing for WIT which would allow to add new enum members or optional fields without being a breaking change.

## Unstable Results

It's not possible to include an unstable typ inside a `Result` or `Option`, as this will create another anonymous type, which is not marked as unstable.
See [this issue](https://github.com/bytecodealliance/wasm-tools/issues/2210) for details. Instead, you can create the result type yourself and mark it as unstable:

```wit
@unstable
variant error {
    other
}

@unstable(feature = tool)
type custom-result = result<list<u8>, error>;
```