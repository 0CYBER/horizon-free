# Hosts

The compiled slices are built for these computers, the game's and, where listed, the dedicated server's:

{{#sdk-hosts}}

On any other computer the launcher builds no native mods and says `E_NATIVE_HOST: no prebuilt game supports this
computer; use a supported host`. Intel Macs and Windows on ARM run the launcher but have no SDK, and a dedicated
server takes native mods on Linux x86-64 only.

Writing a mod needs no compiled slice: the API source in `.horizon/sdk/` is the same on every computer, so
`cargo horizon-mod` writes and checks a mod on any computer `cargo` runs on. Only building it into the game needs the
slices of the computer that plays it.
