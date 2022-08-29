const TCP_PORT = 24818;
const MW_PJ64_PROTO_VERSION = 1;
//TODO generate above constants from Rust code
const DEFAULT_PLAYER_NAME = [0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf];
const SRAM_START = 0xA8000000;

var readBuf = new Buffer(0);
var versionChecked = false;
var playerID = null;
var playerName = null;
var playerNames = [
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
    DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME, DEFAULT_PLAYER_NAME,
];
var itemQueue = [];
var remainingItems = 0;
var sock = new Socket();
sock.on('close', function() {
    throw 'connection to multiworld app lost';
});
sock.connect({host: "127.0.0.1", port: TCP_PORT}, function() {
    const handshake = new ArrayBuffer(1);
    new DataView(handshake).setUint8(0, MW_PJ64_PROTO_VERSION);
    sock.write(new Buffer(new Uint8Array(handshake)), function() {
        sock.on('data', function(buf) {
            var newBuf = new Buffer(readBuf.length + buf.length);
            readBuf.copy(newBuf);
            new Buffer(buf).copy(newBuf, readBuf.length);
            readBuf = newBuf;
            if (!versionChecked && readBuf.length >= 1) {
                // check to make sure the server's protocol version matches ours
                if (readBuf[0] != MW_PJ64_PROTO_VERSION) {
                    sock.close();
                    throw 'version mismatch';
                }
                console.log('Connected to multiworld app');
                versionChecked = true;
                readBuf = readBuf.slice(1);
            }
            var eof = false;
            while (versionChecked && !eof) {
                if (remainingItems > 0) {
                    if (readBuf.length >= 2) {
                        itemQueue.push(readBuf.readUInt16BE(0));
                        readBuf = readBuf.slice(2);
                        remainingItems -= 1;
                    } else {
                        eof = true;
                    }
                } else {
                    if (readBuf.length >= 1) {
                        switch (readBuf.readUInt8(0)) {
                            case 0: // ServerMessage::ItemQueue
                                if (readBuf.length >= 9) {
                                    if (readBuf.readUInt32BE(1) != 0) {
                                        sock.close();
                                        throw 'more than u32::MAX_VALUE items';
                                    }
                                    itemQueue = [];
                                    remainingItems = readBuf.readUInt32BE(5);
                                    readBuf = readBuf.slice(9);
                                } else {
                                    eof = true;
                                }
                                break;
                            case 1: // ServerMessage::GetItem
                                remainingItems = 1;
                                readBuf = readBuf.slice(1);
                                break;
                            case 2: // ServerMessage::PlayerName
                                if (readBuf.length >= 10) {
                                    playerNames[readBuf.readUInt8(1)] = readBuf.slice(2, 10);
                                    readBuf = readBuf.slice(10);
                                } else {
                                    eof = true;
                                }
                                break;
                            default:
                                sock.close();
                                throw 'unknown server command';
                        }
                    } else {
                        eof = true;
                    }
                }
            }
        });
        events.ondraw(function() {
            // read player ID
            var zeldaz_rdram = mem.getblock(ADDR_ANY_RDRAM.start + 0x11a5d0 + 0x1c, 6);
            var coopContextAddr = null;
            if (zeldaz_rdram[0] == 0x5a && zeldaz_rdram[1] == 0x45 && zeldaz_rdram[2] == 0x4c && zeldaz_rdram[3] == 0x44 && zeldaz_rdram[4] == 0x41 && zeldaz_rdram[5] == 0x5a) {
                var randoContextAddr = mem.u32[ADDR_ANY_RDRAM.start + 0x1c6e90 + 0x15d4];
                if (randoContextAddr >= 0x80000000 && randoContextAddr != 0xffffffff) {
                    var newCoopContextAddr = mem.u32[randoContextAddr];
                    if (newCoopContextAddr >= 0x80000000 && newCoopContextAddr != 0xffffffff) {
                        //TODO COOP_VERSION check
                        coopContextAddr = newCoopContextAddr;
                        var newPlayerID = mem.u8[newCoopContextAddr + 0x4];
                        if (newPlayerID !== playerID) {
                            const playerIdPacket = new ArrayBuffer(2);
                            var playerIdPacketView = new DataView(playerIdPacket);
                            playerIdPacketView.setUint8(0, 0); // message: player ID changed
                            playerIdPacketView.setUint8(1, newPlayerID);
                            sock.write(new Buffer(new Uint8Array(playerIdPacket)));
                            playerID = newPlayerID;
                            if (playerName !== null) {
                                playerNames[playerID] = playerName;
                            }
                        }
                    }
                }
            }
            // sync player names
            var newPlayerName;
            var zeldaz_sram = mem.getblock(SRAM_START + 0x0020 + 0x1c, 6);
            if (playerID === null) {
                // player ID null, setting default player name
                newPlayerName = DEFAULT_PLAYER_NAME;
            } else if (zeldaz_sram[0] == 0x5a && zeldaz_sram[1] == 0x45 && zeldaz_sram[2] == 0x4c && zeldaz_sram[3] == 0x44 && zeldaz_sram[4] == 0x41 && zeldaz_sram[5] == 0x5a) {
                // get own player name from save file
                newPlayerName = mem.getblock(SRAM_START + 0x0020 + 0x0024, 8);
                // always fill player names in co-op context (some player names may go missing seemingly at random while others stay intact, so this has to run every frame)
                if (coopContextAddr !== null) {
                    for (var world = 1; world < 256; world++) {
                        for (var c = 0; c < 8; c++) {
                            mem.u8[coopContextAddr + 0x14 + world * 0x8 + c] = playerNames[world][c];
                        }
                    }
                }
            } else {
                // file 1 does not exist, reset player name
                newPlayerName = DEFAULT_PLAYER_NAME;
            }
            var playerNameChanged = false;
            if (playerName === null) {
                playerNameChanged = true;
            } else {
                for (var c = 0; c < 8; c++) {
                    if (newPlayerName[c] != playerName[c]) {
                        playerNameChanged = true;
                        break;
                    }
                }
            }
            if (playerNameChanged) {
                const playerNamePacket = new ArrayBuffer(9);
                var playerNamePacketView = new DataView(playerNamePacket);
                playerNamePacketView.setUint8(0, 1); // message: player name changed
                for (var c = 0; c < 8; c++) {
                    playerNamePacketView.setUint8(c + 1, newPlayerName[c]);
                }
                sock.write(new Buffer(new Uint8Array(playerNamePacket)));
                playerName = newPlayerName;
            }
            if (playerID !== null && coopContextAddr !== null) {
                // send item
                var outgoingKey = mem.u32[coopContextAddr + 0xc];
                if (outgoingKey != 0) {
                    var kind = mem.u16[coopContextAddr + 0x10];
                    var player = mem.u8[coopContextAddr + 0x13];
                    if (player == playerID && kind != 0xca) {
                        //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for myself");
                    } else if (outgoingKey == 0xff05ff) {
                        //Debug($"P{this.playerID}: Found an item {kind} for player {player} sent via network, ignoring");
                    } else {
                        //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for player {player}");
                        const sendItemPacket = new ArrayBuffer(8);
                        var sendItemPacketView = new DataView(sendItemPacket);
                        sendItemPacketView.setUint8(0, 2); // message: send item
                        sendItemPacketView.setUint32(1, outgoingKey);
                        sendItemPacketView.setUint16(5, kind);
                        sendItemPacketView.setUint8(7, player);
                        sock.write(new Buffer(new Uint8Array(sendItemPacket)));
                    }
                    mem.u32[coopContextAddr + 0xc] = 0;
                    mem.u16[coopContextAddr + 0x10] = 0;
                    mem.u16[coopContextAddr + 0x12] = 0;
                }
                // receive item
                var stateLogo = mem.u32[ADDR_ANY_RDRAM.start + 0x11f200];
                var stateMain = mem.s8[ADDR_ANY_RDRAM.start + 0x11b92f];
                var stateMenu = mem.s8[ADDR_ANY_RDRAM.start + 0x1d8dd5];
                if (stateLogo != 0x802c5880 && stateLogo != 0 && stateMain != 1 && stateMain != 2 && stateMenu == 0) {
                    if (mem.u16[coopContextAddr + 0x8] == 0) {
                        var internalCount = mem.u16[ADDR_ANY_RDRAM.start + 0x11a5d0 + 0x90];
                        var externalCount = itemQueue.length;
                        if (internalCount < externalCount) {
                            var item = itemQueue[internalCount];
                            //Debug($"P{this.playerID}: Received an item {item} from another player");
                            mem.u16[coopContextAddr + 0x8] = item;
                            mem.u16[coopContextAddr + 0x6] = item == 0xca ? (playerID == 1 ? 2 : 1) : playerID;
                        } else if (internalCount > externalCount) {
                            console.log('warning: gap in received items: internal count is ' + internalCount + ' but external queue is ' + itemQueue);
                        }
                    }
                }
            }
        });
    });
});
