# WIT

It is not possible to add a new interface which is unstable with only unstable members.
So if introducing a new interface which you want, put it behind a `since` gate and make all members `unstable`.
Adding a new parameter to a struct, even if it optional, is a breaking change from the ABI perspective at the moment.
So it requires either introducing a new struct and methods associated with it, or a new major version of the wit World.
There is ideas for a more looser subtyping for WIT which would allow to add new enum members or optional fields without being a breaking change.
