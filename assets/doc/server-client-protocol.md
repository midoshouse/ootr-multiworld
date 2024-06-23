This document specifies the network protocol used to communicate between the Mido's House Multiworld server (`ootrmwd`) and app (`multiworld-gui`).

The protocol is versioned with respect to breaking changes from a client's perspective, e.g. new client→server messages may be added without a protocol version change but not new server→client messages. The protocol version corresponds to the major [release](https://github.com/midoshouse/ootr-multiworld/releases) version of the first-party client. The current version is 16. Each previous version remains available and supported until 6 months have passed since the last time a client with this version has connected or disconnected, or until 2 years have passed since the following version was released, whichever happens first.

This API is available at `wss://mw.midos.house/v16`. All messages are binary [WebSocket](https://en.wikipedia.org/wiki/WebSocket) messages. The message kind is determined from the direction of the message and the first byte according to the following two sections. The data types that appear in the messages are defined in the third section below. All data types are [big-endian](https://en.wikipedia.org/wiki/Endianness). A message that contains multiple fields of data or a compound data type is simply represented as each field in sequence, so you may have to read one field to know where the next field starts. Names of messages, of message fields, and of compound data types are listed here for reference only, they do not appear in the binary forms of the messages themselves.

# Server→Client

## `0x00` Ping

This message contains no data (other than the message kind, i.e. its length is always exactly 1 byte). The client must reply with a Ping message of its own. This message is sent by the server every 30 seconds; clients may wish to consider it a network error if no message is received for 60 seconds.

## `0x01` StructuredError

This message has a single one-byte field which defines the type of error that occurred. These errors should not be considered fatal: the client may handle the error, e.g. by displaying it to the user, and then continue the network session normally.

* `0x00` WrongPassword: The client attempted to join a room but sent the wrong password.
* `0x01` RoomExists: The client attempted to create a room with a name that's already taken by another room.
* `0x02` NoMidosHouseAccountDiscord: The client attempted to sign into Mido's House using Discord but there is no Mido's House account associated with the given Discord account.
* `0x03` NoMidosHouseAccountRaceTime: The client attempted to sign into Mido's House using racetime.gg but there is no Mido's House account associated with the given racetime.gg account.
* `0x04` SessionExpiredDiscord: The client attempted to sign in with an expired Discord session token.
* `0x05` SessionExpiredRaceTime: The client attempted to sign in with an expired racetime.gg session token.
* `0x06` ConflictingItemKinds: Clients have reported multiple different items from the same location in the same world. This is [a known issue](https://github.com/midoshouse/ootr-multiworld/issues/43) which is currently being investigated and needs more data — the client should offer to send recent logs, if any, to the developer of the client.

Additional error types may be added without a major version bump, so clients should treat any unknown error type as a generic fatal error.

## `0x02` OtherError

This message has a single [string](#string) field containing an error message. The client should display the message to the user and terminate the connection — usually the server will also terminate the connection.

## `0x03` EnterLobby

Moves the client into the lobby. The server may send this message under several circumstances, including but not limited to after receiving a new connection, if the client leaves a room, if the client is kicked from a room, or if the room the client is in is deleted. This message has a single field (the room list) which is a [map](#map) where each key is an 8-byte number (the room ID) and each value consists of a [string](#string) (the room name) followed by a [Boolean](#boolean) indicating whether this room requires a password.

## `0x04` NewRoom

Sent to a client in the lobby to indicate that a new room has been added to the list, or that an existing room's data has changed. This may be sent because the room was newly created or renamed, but also because the client has signed in, making a private room visible or removing the password requirement from an existing room. Consists of the following fields:

* `id`: The room ID, an 8-byte number. Uniquely identifies this room, indicating whether this is a newly visible room or a change to an existing room's data.
* `name`: The room name, a [string](#string).
* `password_required`: Whether a password is required to join this room, a [Boolean](#string).

## `0x05` DeleteRoom

Sent to a client in the lobby to indicate that a room has been deleted, or is no longer visible to the client. This message has a single field, an 8-byte number indicating the ID of the removed room.

## `0x06` EnterRoom

Moves the client into a room. Consists of the following fields:

* `room_id`: The room ID, an 8-byte number.
* `players`: A [list](#list) of [players](#player) describing the clients in this room which are associated with a world.
* `num_unassigned_clients`: A 1-byte number of clients in this room which are not associated with any world, including the client this message is sent to.
* `autodelete_delta`: A [duration](#duration) specifying how long the server waits to automatically delete this room after the last item is sent to it.
* `allow_send_all`: A [Boolean](#boolean) indicating whether the feature to send all remaining items from a world using a spoiler log is available in this room.

## `0x07` PlayerId

Sent to a client in a room to indicate that a client which previously had no world has claimed a world. This message is sent to all clients in the room, including the client that claimed the world. Consists of the world number as a 1-byte number which will never be zero.

## `0x08` ResetPlayerId

Sent to a client in a room to indicate that a client which previously had a world no longer has an assigned world. This message is sent to all clients in the room, including the client that lost the world assignment. Consists of the client's previous world number as a 1-byte number which will never be zero.

## `0x09` ClientConnected

Sent to a client in a room to indicate that a new client has connected. The client should be considered to have no associated world. This message contains no data.

## `0x0a` PlayerDisconnected

Sent to a client in a room to indicate that a client which previously had a world has left the room. Consists of the client's previous world number as a 1-byte number which will never be zero.

## `0x0b` UnregisteredClientDisconnected

Sent to a client in a room to indicate that a client which previously had no world has left the room. This message contains no data.

## `0x0c` PlayerName

Sent to a client in a room to indicate that a client has changed its filename. This message is sent to all clients in the room, including the client that changed its name. Consists of the client's world number as a 1-byte number which will never be zero, followed by the client's new [filename](#filename).

## `0x0d` ItemQueue

Sent to a client in a room to define the full list of items that have been sent to it, including ones that have already been received. This may be sent multiple times, in which case the previous state of the incoming item queue should be replaced with this data. Consists of a [list](#list) of 2-byte numbers corresponding to the get item IDs defined by the randomizer. Note that get item ID `0x00ca` (Triforce Piece) should be treated specially.

## `0x0e` GetItem

Sent to a client in a room to add an item to the end of its incoming item queue. Consists of a 2-byte number corresponding to a get item ID defined by the randomizer. Note that get item ID `0x00ca` (Triforce Piece) should be treated specially.

## `0x0f` AdminLoginSuccess

Sent to a client in the lobby which has signed in as a MH MW administrator. This message has a single field (the active connections list) which is a [map](#map) where each key is an 8-byte number (a room ID) and each value consists of a [list](#list) of [players](#player) in that room followed by a 1-byte number of clients in that room which are not associated with any world.

## `0x10` Goodbye

Notifies the client that the connection will be dropped by the server. Typically preceded by an error message indicating the reason. The client should consider this server session to be ended even if the underlying WebSocket connection is not closed by the server.

## `0x11` PlayerFileHash

Sent to a client in a room to inform it about the file hash loaded by a client. This may be used to provide additional information to the user in the event that a WrongFileHash message is received. This message is sent to all clients in the room, including the client that reported its file hash. Consists of the client's world number as a 1-byte number which will never be zero, followed by the client's new [file hash](#file-hash).

## `0x12` AutoDeleteDelta

Sent to a client in a room when the duration of inactivity after which this room will be automatically deleted is changed. Consists of the new [duration](#duration).

## `0x13` RoomsEmpty

Sent to a client in the lobby which has sent the WaitUntilEmpty message once there are no players with claimed worlds in any of the rooms. This message is deprecated and will be removed in a future version. It contains no data.

## `0x14` WrongFileHash

Sent to a client in a room if it reports a file hash that doesn't match the file hash set for this room, either as previously reported by a client, or as set by the Mido's House integration for a tournament room. Consists of the [file hash](#file-hash) required for this room, followed by the [file hash](#file-hash) reported by the client.

## `0x15` ProgressiveItems

Sent to a client in a room to update the progressive item state for a client. This message is sent to all clients in the room, including the client whose progressive item state is being updated, though the game currently ignores the player's own progressive item state. Consists of the client's world number as a 1-byte number which will never be zero, followed by the progressive item state, which is 4 bytes of data that should be transparently forwarded to the game. See the documentation for `MW_PROGRESSIVE_ITEMS_ENABLE` and `MW_PROGRESSIVE_ITEMS_STATE` in [the co-op context docs](https://github.com/OoTRandomizer/OoT-Randomizer/blob/Dev/Notes/coop-ctx.md) for details.

## `0x16` LoginSuccess

Confirms to the client that its LoginApiKey, LoginDiscord, or LoginRaceTime message was successful. This message contains no data.

## `0x17` WorldTaken

Sent to a client in a room if it attempts to claim a world number that is currently assigned to a different client in the room. The client should give the user the option to either kick the other client or to leave the room, while also checking for a different rom being loaded to notify the server of a changed world number. Consists of the world number in question as a 1-byte number which will never be zero.

## `0x18` WorldFreed

Sent after a WorldTaken message to indicate that the world is no longer taken. This message contains no data.

## `0x19` MaintenanceNotice

Notifies the client about future server downtime due to maintenance. This maintenance notice should be considered in effect for the rest of the connection session. Consists of the start of the maintenance window as a [date and time](#date-and-time), followed by the estimated [duration](#duration) of the maintenance window.

# Client→Server

## `0x00` Ping

This message contains no data (other than the message kind, i.e. its length is always exactly 1 byte). Must be sent in response to a Ping message from the server. The server may consider it a network error if this is not done within 60 seconds.

## `0x01` JoinRoom

Attempt to join the given room. If this is successful, the server will move the client into the room using an EnterRoom message. Consists of the following fields:

* `id`: The room ID, an 8-byte number.
* `password`: The room password, an [optional](#optional) [string](#string). The presence of this field should correspond to whether this room requires a password, as reported by the server in the EnterLobby or NewRoom message that most recently updated this room's data. An empty password is considered distinct from no password.

## `0x02` CreateRoom

Attempt to create a new room. If this is successful, the server will move the client into the newly created room using an EnterRoom message. Consists of the following fields:

* `name`: The name with which this room is displayed in the public room list, a [string](#string) which must not be empty, has a maximum length of 64 [Unicode scalar values](https://www.unicode.org/glossary/#unicode_scalar_value), and must not contain U+0 NULL. Creating the room will fail if there is already a public room with the same name.
* `password`: The password that will be required to enter the room, a [string](#string) with a maximum length of 64 [Unicode scalar values](https://www.unicode.org/glossary/#unicode_scalar_value) which must not contain U+0 NULL. May be empty.

## `0x03` LoginApiKey

Attempt to sign into a Mido's House account with a Mido's House API key. Consists of the API key, a [string](#string). API keys are issued by @fenhl at their discretion upon request. Signing into a Mido's House account gives the client access to invite-only rooms, such as tournament rooms.

## `0x04` Stop

Attempt to stop the multiworld server. Note that the server will usually be restarted immediately by [systemd](https://en.wikipedia.org/wiki/Systemd). Requires being signed in as a MH MW administrator. This message is deprecated and will be removed in a future version. It contains no data.

## `0x05` PlayerId

Updates the world number for this client. May only be sent while in a room. Consists of the world number as a 1-byte number which must not be zero.

## `0x06` ResetPlayerId

Unassigns the world number from this client. May only be sent while in a room. This message contains no data.

## `0x07` PlayerName

Updates the player name for this client. May only be sent while in a room. Consists of the new [filename](#filename). This message may be used with an empty filename (8 spaces) to reset the player name.

## `0x08` SendItem

Notifies the server that the client has collected an item. May only be sent while in a room. The client is encouraged to send this message even if the player has found an item for themself, which helps the server report more accurate progressive item states to the other clients. This can be accomplished by enabling `MW_SEND_OWN_ITEMS` in the co-op context. Consists of the following fields:

* `key`: The override key identifying the location where the item was found, an 8-byte number. Note that co-op context versions 6 and earlier use 4-byte override keys; in this case, the key should be zero-extended to 8 bytes.
* `kind`: The get item ID of the item as defined by the randomizer, a 2-byte number.
* `target_world`: The world number of the player who should receive the item, a 1-byte number which must not be zero.

## `0x09` KickPlayer

Kick a client which has claimed a world number from the current room. May only be sent while in a room. Consists of the world number as a 1-byte number which must not be zero. The server typically allows any member of a room to kick any other member of the same room. Clients with no claimed world currently can't be kicked.

## `0x0a` DeleteRoom

Immediately deletes the current room. May only be sent while in a room. The server typically allows any member of a room to delete the room; restrictions may be in place for special event rooms created by an admin. This message contains no data.

## `0x0b` Track

Enables restream auto-trackers on <https://oottracker.fenhl.net/> for the given room. Requires being signed in as a MH MW administrator. Consists of the following fields:

* `mw_room`: The room ID, an 8-byte number.
* `tracker_room_name`: The name that will be used for this room on <https://oottracker.fenhl.net/>, a [string](#string). Anyone who knows this name will be able to access the trackers, so this should typically be a randomly generated password.
* `world_count`: The number of worlds in the seed, a 1-byte number which must not be zero.

## `0x0c` SaveData

Updates the client's state using the contents of the save file. Should be sent each time the player opens a save file. The server uses this information to update the progressive item state, as well as for auto-tracking. Consists of the contents of the save data, a blob `0x1450` bytes in length. See <https://wiki.cloudmodding.com/oot/Save_Format> for details.

## `0x0d` SendAll

Requests that all remaining items from the given world be distributed. Useful when a player stops playing the seed but the other players want to continue playing. Consists of the following fields, all of which except for `source_world` should be sourced from the seed's spoiler log:

* `source_world`: The number of the world from which items should be distributed, a 1-byte number which must not be zero.
* `file_hash`: The seed's [file hash](#file-hash).
* `:version`: The seed's randomizer version, a [string](#string).
* `settings`: Information about a subset of each world's settings. Should be sent as a [list](#list) even on versions of the randomizer with no per-world settings. Each element of the list consists of the following fields:
    * `world_count` as a 1-byte number which must not be zero.
    * `lacs_condition` as one byte with one of the following values:
        * `0x00`: `vanilla`
        * `0x01`: `stones`
        * `0x02`: `medallions`
        * `0x03`: `dungeons`
        * `0x04`: `tokens`
        * `0x05`: `hearts`
    * `bridge` as one byte with one of the following values:
        * `0x00`: `open`
        * `0x01`: `vanilla`
        * `0x02`: `stones`
        * `0x03`: `medallions`
        * `0x04`: `dungeons`
        * `0x05`: `tokens`
        * `0x06`: `hearts`
        * `0x07`: `random`
    * `shuffle_ganon_bosskey` as one byte with one of the following values:
        * `0x00`: `remove`
        * `0x01`: `vanilla`
        * `0x02`: `dungeon`
        * `0x03`: `regional`
        * `0x04`: `overworld`
        * `0x05`: `any_dungeon`
        * `0x06`: `keysanity`
        * `0x07`: `on_lacs`
        * `0x08`: `stones`
        * `0x09`: `medallions`
        * `0x0a`: `dungeons`
        * `0x0b`: `tokens`
        * `0x0c`: `hearts`
    * `keyring_give_bk` as a [Boolean](#boolean)
    * `free_bombchu_drops` as a [Boolean](#boolean)
    * `correct_chest_sizes` as a [Boolean](#boolean). Set to false for randomizer versions which don't have this setting anymore due to being replaced with `correct_chest_appearances`.
    * `correct_chest_appearances` as an [optional](#optional) byte with one of the following values if present:
        * `0x00`: `off`
        * `0x01`: `classic`
        * `0x02`: `textures`
        * `0x03`: `both`
    * `minor_items_as_major_chest`, consisting of the following fields:
        * `bombchus`, a [Boolean](#boolean)
        * `shields`, a [Boolean](#boolean)
        * `capacity`, a [Boolean](#boolean)
    * `invisible_chests`, a [Boolean](#boolean)
* `randomized_settings`: Information about a subset of each world's randomized settings, a [list](#list) with each element consisting of the following fields:
    * `bridge` as one byte with the same values as the `bridge` field of the `settings` field
* `locations`: A [list](#list) of one [map](#map) per world, where each key is a location name as a [string](#string) and each value consists of the following fields:
    * `player`: The world number of the item's recipient, a 1-byte number which must not be zero
    * `item`: The name of the item, a [string](#string)
    * `model`: An [optional](#optional) [string](#string), present if this item is an ice trap in a location where the cloak is relevant, defining the item it is cloaked as

Some of these fields are unused and will be removed in a future version. As of now, they still need to be present and have valid values. The following fields are actually used:

* `source_world`
* `file_hash`
* `:version`
* the `keyring_give_bk` setting
* `locations`

## `0x0e` SaveDataError

Should not be sent by third-party clients.

## `0x0f` FileHash

Updates the file hash for this client. May only be sent while in a room. Consists of the new [file hash](#file-hash).

## `0x10` AutoDeleteDelta

Changes the duration of inactivity after which this room will be automatically deleted. May only be sent while in a room. Consists of the new [duration](#duration).

## `0x11` WaitUntilEmpty

Requests that the server send a RoomsEmpty message once there are no clients with assigned world numbers in any room. May only be sent while in the lobby and only after siging in as a MH MW administrator. This message is deprecated and will be removed in a future version. It contains no data.

## `0x12` LoginDiscord

Attempt to sign into a Mido's House account with an [OAuth](https://en.wikipedia.org/wiki/OAuth) bearer token from [Discord](https://discord.com/). May only be sent while in the lobby. Consists of the bearer token, a [string](#string). Signing into a Mido's House account gives the client access to invite-only rooms, such as tournament rooms.

## `0x13` LoginRaceTime

Attempt to sign into a Mido's House account with an [OAuth](https://en.wikipedia.org/wiki/OAuth) bearer token from [racetime.gg](https://racetime.gg/). May only be sent while in the lobby. Consists of the bearer token, a [string](#string). Signing into a Mido's House account gives the client access to invite-only rooms, such as tournament rooms.

## `0x14` LeaveRoom

Leave the current room and go to the lobby. May only be sent while in a room. This message contains no data.

## `0x15` DungeonRewardInfo

Notifies the server about information on the locations of dungeon rewards (medallions and spiritual stones) that the player has obtained from the game. Should be sent each time the player gains new relevant information, whether by opening the dungeon reward info screen in the pause menu or by other means such as reading a hint or collecting the reward itself. The server only uses this information for auto-tracking, but should always be sent regardless. May only be sent while in a room. Consists of the following fields:

* `reward`: 2 bytes with one of the following values:
    * `0x0000`: Light Medallion
    * `0x0001`: Forest Medallion
    * `0x0002`: Fire Medallion
    * `0x0003`: Water Medallion
    * `0x0004`: Shadow Medallion
    * `0x0005`: Spirit Medallion
    * `0x0100`: Kokiri Emerald
    * `0x0101`: Goron Ruby
    * `0x0102`: Zora Sapphire
* `world`: The number of the world where the dungeon reward is located, a 1-byte number which must not be zero.
* `area`: The hint area where the dungeon reward is located, 1 byte with one of the following values:
    * `0x00`: Root (Link's Pocket)
    * `0x01`: Hyrule Field
    * `0x02`: Lon Lon Ranch
    * `0x03`: Market
    * `0x04`: Temple of Time
    * `0x05`: Hyrule Castle
    * `0x06`: Outside Ganon's Castle
    * `0x07`: Inside Ganon's Castle
    * `0x08`: Kokiri Forest
    * `0x09`: Deku Tree
    * `0x0a`: Lost Woods
    * `0x0b`: Sacred Forest Meadow
    * `0x0c`: Forest Temple
    * `0x0d`: Death Mountain Trail
    * `0x0e`: Dodongo's Cavern
    * `0x0f`: Goron City
    * `0x10`: Death Mountain Crater
    * `0x11`: Fire Temple
    * `0x12`: Zora River
    * `0x13`: Zora's Domain
    * `0x14`: Zora's Fountain
    * `0x15`: Jabu Jabu's Belly
    * `0x16`: Ice Cavern
    * `0x17`: Lake Hylia
    * `0x18`: Water Temple
    * `0x19`: Kakariko Village
    * `0x1a`: Bottom of the Well
    * `0x1b`: Graveyard
    * `0x1c`: Shadow Temple
    * `0x1d`: Gerudo Valley
    * `0x1e`: Gerudo Fortress
    * `0x1f`: Thieves' Hideout
    * `0x20`: Gerudo Training Ground
    * `0x21`: Haunted Wasteland
    * `0x22`: Desert Colossus
    * `0x23`: Spirit Temple

## `0x16` CurrentScene

Notifies the server about the scene ID the player is currently in. Should be sent each time this changes, regardless of whether the client is in the lobby or in a room. Currently only used for special events, but should always be sent regardless. Consists of the new scene ID as a 1-byte number.

# Data types

## Boolean

One byte with the value `0x01` for true or `0x00` for false.

## Date and time

A signed 8-byte number of whole seconds since the [Unix epoch](https://en.wikipedia.org/wiki/Unix_time), followed by an unsigned 4-byte number of subsecond nanoseconds.

## Duration

An 8-byte number of whole seconds followed by a 4-byte number of subsecond nanoseconds. Durations are never negative.

## File hash

A sequence of 5 bytes, with each byte representing a hash icon as an index into [the `HASH_ICONS` list in Spoiler.py](https://github.com/OoTRandomizer/OoT-Randomizer/blob/511d98668132a46b52ff959455724dbb83d3d82f/Spoiler.py#L22-L55). This identifies a seed using the 5 icons displayed at the top of the file select screen.

## Filename

A sequence of 8 bytes in OoT's internal encoding, padded with spaces (`0xdf`) at the end. Note that the encoding used for filenames is distinct from the encoding used for other text in OoT. See [sample code defining the encoding](https://github.com/midoshouse/ootr-multiworld/blob/ce721eb59301560a0e7c247a7e49cc038d02b335/crate/multiworld/src/lib.rs#L250-L267).

## List

Consists of an 8-byte number defining the number of elements in the list, followed by that many elements.

## Map

Consists of an 8-byte number defining the number of elements in the map, followed by that many elements, where each element is a key followed by a value. Each key is guaranteed to be present at most once in the map.

## Optional

Starts with a [Boolean](#boolean). If the Boolean is true, it is followed by a value of the given type. If it's false, there is no additional data.

## Player

Consists of the following fields:

* `world`: The world number as a 1-byte number. Must not be zero.
* `name`: The player name as a [filename](#filename).
* `file_hash`: The [optional](#optional) [file hash](#file-hash) reported by the player.

## String

Consists of an 8-byte number defining the number of bytes of the payload, followed by the payload which is [UTF-8](https://en.wikipedia.org/wiki/UTF-8)-encoded text. Strings are not null-terminated and strings sent by the server may contain internal null characters.
