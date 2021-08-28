This is an alternative implementation of [multiworld](https://wiki.ootrandomizer.com/index.php?title=Multiworld) for [the Ocarina of Time randomizer](https://ootrandomizer.com/), intended to improve upon [the existing implementation](https://github.com/TestRunnerSRL/bizhawk-co-op) by breaking compatibility with it. It is currently experimental and not ready to be used.

# Goals

* Reduce code complexity by focusing on OoTR
* Establish connections through a dedicated server, skipping the need for port forwarding or Hamachi
* BizHawk client written as an external tool, improving performance compared to Lua scripting
* Additional client for Project64
* Allow the server to use data about obtained items for auto-tracking for restreams
* Easier configuration: read player name and number from the game

# Credits

* Some code based on [Bizhawk Shuffler 2](https://github.com/authorblues/bizhawk-shuffler-2)
