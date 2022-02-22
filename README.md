This is an alternative implementation of [multiworld](https://wiki.ootrandomizer.com/index.php?title=Multiworld) for [the Ocarina of Time randomizer](https://ootrandomizer.com/), intended to improve upon [the existing implementation](https://github.com/TestRunnerSRL/bizhawk-co-op) by breaking compatibility with it. It is currently experimental but ready to be tested.

# Goals

- [x] Reduce code complexity by focusing on OoTR
- [x] Establish connections through a dedicated server, skipping the need for port forwarding or Hamachi
- [ ] Use a WebSocket connection if direct TCP is not available or encryption is desired
- [ ] Still allow players to host locally (without a dedicated server) if desired
- [x] BizHawk client written as an external tool, improving performance compared to Lua scripting
- [ ] Additional client for Project64
- [ ] Automatically keep room list clean by closing rooms after a period of inactivity
- [ ] Backup system to allow players to restore closed rooms or move to a different host
- [ ] Allow the server to use data about obtained items for auto-tracking for restreams
- [x] Easier configuration: read player name and number from the game

# Credits

* Some code based on [Bizhawk Shuffler 2](https://github.com/authorblues/bizhawk-shuffler-2)
