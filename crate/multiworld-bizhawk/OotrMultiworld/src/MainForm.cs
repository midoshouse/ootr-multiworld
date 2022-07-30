using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Forms;
using BizHawk.Client.Common;
using BizHawk.Client.EmuHawk;

namespace Net.Fenhl.OotrMultiworld {
    internal class Native {
        [DllImport("multiworld")] internal static extern LobbyClientResult connect_ipv4();
        [DllImport("multiworld")] internal static extern LobbyClientResult connect_ipv6();
        [DllImport("multiworld")] internal static extern void lobby_client_result_free(IntPtr lobby_client_res);
        [DllImport("multiworld")] internal static extern bool lobby_client_result_is_ok(LobbyClientResult lobby_client_res);
        [DllImport("multiworld")] internal static extern LobbyClient lobby_client_result_unwrap(IntPtr lobby_client_res);
        [DllImport("multiworld")] internal static extern void lobby_client_free(IntPtr lobby_client);
        [DllImport("multiworld")] internal static extern StringHandle lobby_client_result_debug_err(IntPtr lobby_client_res);
        [DllImport("multiworld")] internal static extern void string_free(IntPtr s);
        [DllImport("multiworld")] internal static extern ulong lobby_client_num_rooms(LobbyClient lobby_client);
        [DllImport("multiworld")] internal static extern StringHandle lobby_client_room_name(LobbyClient lobby_client, ulong i);
        [DllImport("multiworld")] internal static extern StringResult lobby_client_try_recv_new_room(LobbyClient lobbyClient);
        [DllImport("multiworld")] internal static extern void string_result_free(IntPtr str_res);
        [DllImport("multiworld")] internal static extern bool string_result_is_ok(StringResult str_res);
        [DllImport("multiworld")] internal static extern StringHandle string_result_unwrap(IntPtr str_res);
        [DllImport("multiworld")] internal static extern StringHandle string_result_debug_err(IntPtr str_res);
        [DllImport("multiworld")] internal static extern RoomClientResult lobby_client_room_connect(IntPtr lobby_client, OwnedStringHandle room_name, OwnedStringHandle password);
        [DllImport("multiworld")] internal static extern void room_client_result_free(IntPtr room_client_res);
        [DllImport("multiworld")] internal static extern bool room_client_result_is_ok(RoomClientResult room_client_res);
        [DllImport("multiworld")] internal static extern RoomClient room_client_result_unwrap(IntPtr room_client_res);
        [DllImport("multiworld")] internal static extern void room_client_free(IntPtr room_client);
        [DllImport("multiworld")] internal static extern StringHandle room_client_result_debug_err(IntPtr room_client_res);
        [DllImport("multiworld")] internal static extern UnitResult room_client_set_player_id(RoomClient room_client, byte id);
        [DllImport("multiworld")] internal static extern void unit_result_free(IntPtr unit_res);
        [DllImport("multiworld")] internal static extern bool unit_result_is_ok(UnitResult unit_res);
        [DllImport("multiworld")] internal static extern StringHandle unit_result_debug_err(IntPtr unit_res);
        [DllImport("multiworld")] internal static extern UnitResult room_client_reset_player_id(RoomClient room_client);
        [DllImport("multiworld")] internal static extern UnitResult room_client_set_player_name(RoomClient room_client, IntPtr name);
        [DllImport("multiworld")] internal static extern StringHandle room_client_format_state(RoomClient room_client);
        [DllImport("multiworld")] internal static extern OptMessageResult room_client_try_recv_message(RoomClient room_client);
        [DllImport("multiworld")] internal static extern void opt_message_result_free(IntPtr opt_msg_res);
        [DllImport("multiworld")] internal static extern bool opt_message_result_is_ok_some(OptMessageResult opt_msg_res);
        [DllImport("multiworld")] internal static extern ServerMessage opt_message_result_unwrap_unwrap(IntPtr opt_msg_res);
        [DllImport("multiworld")] internal static extern void message_free(IntPtr msg);
        [DllImport("multiworld")] internal static extern bool opt_message_result_is_err(OptMessageResult opt_msg_res);
        [DllImport("multiworld")] internal static extern StringHandle opt_message_result_debug_err(IntPtr opt_msg_res);
        [DllImport("multiworld")] internal static extern byte message_effect_type(ServerMessage msg);
        [DllImport("multiworld")] internal static extern byte message_player_id(ServerMessage msg);
        [DllImport("multiworld")] internal static extern IntPtr message_player_name(ServerMessage msg);
        [DllImport("multiworld")] internal static extern void room_client_apply_message(RoomClient room_client, IntPtr msg);
        [DllImport("multiworld")] internal static extern UnitResult room_client_send_item(RoomClient room_client, uint key, ushort kind, byte target_world);
        [DllImport("multiworld")] internal static extern ushort room_client_item_queue_len(RoomClient room_client);
        [DllImport("multiworld")] internal static extern ushort room_client_item_kind_at_index(RoomClient room_client, ushort index);
        [DllImport("multiworld")] internal static extern IntPtr room_client_get_player_name(RoomClient room_client, byte world);
    }

    internal class StringHandle : SafeHandle {
        internal StringHandle() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        public string AsString() {
            int len = 0;
            while (Marshal.ReadByte(this.handle, len) != 0) { len += 1; }
            byte[] buffer = new byte[len];
            Marshal.Copy(this.handle, buffer, 0, buffer.Length);
            return Encoding.UTF8.GetString(buffer);
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.string_free(this.handle);
            }
            return true;
        }
    }

    internal class LobbyClient : SafeHandle {
        internal LobbyClient() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.lobby_client_free(this.handle);
            }
            return true;
        }

        internal ulong NumRooms() => Native.lobby_client_num_rooms(this);
        internal StringHandle RoomName(ulong i) => Native.lobby_client_room_name(this, i);
        internal StringResult TryRecvNewRoom() => Native.lobby_client_try_recv_new_room(this);

        internal RoomClientResult CreateJoinRoom(string roomName, string password) {
            using (var nameHandle = new OwnedStringHandle(roomName)) {
                using (var passwordHandle = new OwnedStringHandle(password)) {
                    var res = Native.lobby_client_room_connect(this.handle, nameHandle, passwordHandle);
                    this.handle = IntPtr.Zero; // lobby_client_room_connect takes ownership
                    return res;
                }
            }
        }
    }

    internal class LobbyClientResult : SafeHandle {
        internal LobbyClientResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.lobby_client_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOk() => Native.lobby_client_result_is_ok(this);

        internal LobbyClient Unwrap() {
            var lobbyClient = Native.lobby_client_result_unwrap(this.handle);
            this.handle = IntPtr.Zero; // lobby_client_result_unwrap takes ownership
            return lobbyClient;
        }

        internal StringHandle DebugErr() {
            var err = Native.lobby_client_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // lobby_client_result_debug_err takes ownership
            return err;
        }
    }

    internal class OptMessageResult : SafeHandle {
        internal OptMessageResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.opt_message_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOkSome() => Native.opt_message_result_is_ok_some(this);
        internal bool IsErr() => Native.opt_message_result_is_err(this);

        internal ServerMessage UnwrapUnwrap() {
            var msg = Native.opt_message_result_unwrap_unwrap(this.handle);
            this.handle = IntPtr.Zero; // opt_message_result_unwrap_unwrap takes ownership
            return msg;
        }

        internal StringHandle DebugErr() {
            var err = Native.opt_message_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // opt_msg_result_debug_err takes ownership
            return err;
        }
    }

    internal class RoomClient : SafeHandle {
        internal RoomClient() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.room_client_free(this.handle);
            }
            return true;
        }

        internal UnitResult SetPlayerID(byte? id) {
            if (id == null) {
                return Native.room_client_reset_player_id(this);
            } else {
                return Native.room_client_set_player_id(this, id.Value);
            }
        }

        internal UnitResult SetPlayerName(List<byte> name) {
            var namePtr = Marshal.AllocHGlobal(8);
            Marshal.Copy(name.ToArray(), 0, namePtr, 8);
            var res = Native.room_client_set_player_name(this, namePtr);
            Marshal.FreeHGlobal(namePtr);
            return res;
        }

        internal List<byte> GetPlayerName(byte world) {
            var name = new byte[8];
            Marshal.Copy(Native.room_client_get_player_name(this, world), name, 0, 8);
            return name.ToList();
        }

        internal StringHandle State() => Native.room_client_format_state(this);
        internal OptMessageResult TryRecv() => Native.room_client_try_recv_message(this);
        internal UnitResult SendItem(uint key, ushort kind, byte targetWorld) => Native.room_client_send_item(this, key, kind, targetWorld);
        internal ushort ItemQueueLen() => Native.room_client_item_queue_len(this);
        internal ushort Item(ushort index) => Native.room_client_item_kind_at_index(this, index);
    }

    internal class RoomClientResult : SafeHandle {
        internal RoomClientResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.room_client_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOk() => Native.room_client_result_is_ok(this);

        internal RoomClient Unwrap() {
            var roomClient = Native.room_client_result_unwrap(this.handle);
            this.handle = IntPtr.Zero; // room_client_result_unwrap takes ownership
            return roomClient;
        }

        internal StringHandle DebugErr() {
            var err = Native.room_client_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // room_client_result_debug_err takes ownership
            return err;
        }
    }

    internal class ServerMessage : SafeHandle {
        internal ServerMessage() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.message_free(this.handle);
            }
            return true;
        }

        internal byte EffectType() => Native.message_effect_type(this);
        internal byte World() => Native.message_player_id(this);

        internal List<byte> Filename() {
            var name = new byte[8];
            Marshal.Copy(Native.message_player_name(this), name, 0, 8);
            return name.ToList();
        }

        internal void Apply(RoomClient roomClient) {
            Native.room_client_apply_message(roomClient, this.handle);
            this.handle = IntPtr.Zero; // room_client_apply_message takes ownership of the message
        }
    }

    internal class StringResult : SafeHandle {
        internal StringResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.string_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOk() => Native.string_result_is_ok(this);

        internal StringHandle Unwrap() {
            var s = Native.string_result_unwrap(this.handle);
            this.handle = IntPtr.Zero; // string_result_unwrap takes ownership
            return s;
        }

        internal StringHandle DebugErr() {
            var err = Native.string_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // string_result_debug_err takes ownership
            return err;
        }
    }

    internal class UnitResult : SafeHandle {
        internal UnitResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.unit_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOk() => Native.unit_result_is_ok(this);

        internal StringHandle DebugErr() {
            var err = Native.unit_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // unit_result_debug_err takes ownership
            return err;
        }
    }

    internal class OwnedStringHandle : SafeHandle {
        internal OwnedStringHandle(string value) : base(IntPtr.Zero, true) {
            var bytes = Encoding.UTF8.GetBytes(value);
            this.handle = Marshal.AllocHGlobal(bytes.Length + 1);
            Marshal.Copy(bytes, 0, this.handle, bytes.Length);
            Marshal.WriteByte(handle, bytes.Length, 0);
        }

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Marshal.FreeHGlobal(this.handle);
            }
            return true;
        }
    }

    [ExternalTool("OoTR multiworld", Description = "Play interconnected Ocarina of Time Randomizer seeds")]
    public sealed class MainForm : ToolFormBase, IExternalToolForm {
        private Label state = new Label();
        private ComboBox rooms = new ComboBox();
        private TextBox password = new TextBox();
        private Button createJoinButton = new Button();
        private Label roomState = new Label();

        private LobbyClient? lobbyClient;
        private RoomClient? roomClient;
        private uint? coopContextAddr;
        private byte? playerID;
        private List<byte> playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };

        public ApiContainer? _apiContainer { get; set; }
        private ApiContainer APIs => _apiContainer ?? throw new NullReferenceException();

        public override bool BlocksInputWhenFocused { get; } = false;
        protected override string WindowTitleStatic => "OoTR Multiworld for BizHawk";

        public override bool AskSaveChanges() => true; //TODO warn before leaving an active game?

        public MainForm() {
            SuspendLayout();
            this.ClientSize = new Size(509, 256);

            this.state.TabIndex = 0;
            this.state.AutoSize = true;
            this.state.Location = new Point(12, 9);
            this.Controls.Add(this.state);

            this.rooms.TabIndex = 1;
            this.rooms.Location = new Point(12, 42);
            this.rooms.Size = new Size(485, 25);
            this.rooms.Enabled = false;
            this.rooms.Items.Add("Loading room list…");
            this.rooms.SelectedIndex = 0;
            this.rooms.AutoCompleteMode = AutoCompleteMode.Append;
            this.rooms.AutoCompleteSource = AutoCompleteSource.ListItems;
            this.rooms.TextChanged += (s, e) => {
                this.LobbyStateChanged();
            };
            this.Controls.Add(this.rooms);

            this.password.TabIndex = 2;
            this.password.Location = new Point(12, 82);
            this.password.Size = new Size(485, 25);
            password.UseSystemPasswordChar = true;
            //TODO (.net 5) add PlaceholderText (“Password”)
            this.password.TextChanged += (s, e) => {
                this.createJoinButton.Enabled = this.rooms.Enabled && this.rooms.Text.Length > 0 && this.password.Text.Length > 0;
            };
            this.Controls.Add(this.password);

            this.createJoinButton.TabIndex = 3;
            this.createJoinButton.Location = new Point(11, 119);
            this.createJoinButton.AutoSize = true;
            this.createJoinButton.Text = "Create/Join";
            this.createJoinButton.Enabled = false;
            this.createJoinButton.Click += (s, e) => {
                if (this.lobbyClient != null) {
                    using (var res = this.lobbyClient.CreateJoinRoom(this.rooms.Text, this.password.Text)) {
                        if (res.IsOk()) {
                            JoinRoom(res.Unwrap());
                        } else {
                            using (var err = res.DebugErr()) {
                                Error(err.AsString());
                            }
                        }
                    }
                }
            };
            this.Controls.Add(this.createJoinButton);

            this.roomState.TabIndex = 4;
            this.roomState.Location = new Point(12, 42);
            this.roomState.AutoSize = true;
            this.roomState.Visible = false;
            this.Controls.Add(this.roomState);

            ResumeLayout(true);
        }

        public override void Restart() {
            APIs.Memory.SetBigEndian(true);
            if ((APIs.GameInfo.GetGameInfo()?.Name ?? "Null") == "Null") {
                this.state.Text = "Please open the ROM…";
                HideUI();
                return;
            }
            this.playerID = null;
            if (this.roomClient != null) {
                ReadPlayerID();
                SyncPlayerNames();
                ShowUI();
            } else if (this.lobbyClient == null) {
                using (var res6 = Native.connect_ipv6()) {
                    if (res6.IsOk()) {
                        OnConnect(res6.Unwrap());
                    } else {
                        using (var res4 = Native.connect_ipv4()) {
                            if (res4.IsOk()) {
                                OnConnect(res4.Unwrap());
                            } else {
                                //TODO TCP connections unavailable, try WebSocket instead. If that fails too, offer self-hosting/direct connections
                                using (var err = res4.DebugErr()) {
                                    this.state.Text = $"error: {err.AsString()}";
                                }
                                this.rooms.Items[0] = "Failed to load room list";
                            }
                        }
                    }
                }
                ShowUI();
            }
        }

        private void OnConnect(LobbyClient lobbyClient) {
            this.lobbyClient = lobbyClient;
            var numRooms = this.lobbyClient.NumRooms();
            this.state.Text = $"Join or create a room:";
            SuspendLayout();
            this.rooms.SelectedItem = null;
            this.rooms.Items.Clear();
            for (ulong i = 0; i < numRooms; i++) {
                this.rooms.Items.Add(this.lobbyClient.RoomName(i).AsString());
            }
            this.rooms.Enabled = true;
            ResumeLayout(true);
        }

        public override void UpdateValues(ToolFormUpdateType type) {
            if (type != ToolFormUpdateType.PreFrame && type != ToolFormUpdateType.FastPreFrame) {
                return;
            }
            if ((APIs.GameInfo.GetGameInfo()?.Name ?? "Null") == "Null") {
                return;
            }
            if (this.lobbyClient != null) {
                using (var res = this.lobbyClient.TryRecvNewRoom()) {
                    if (res.IsOk()) {
                        using (var newRoom = res.Unwrap()) {
                            var name = newRoom.AsString();
                            if (name.Length > 0) {
                                this.rooms.Items.Add(newRoom.AsString());
                                this.LobbyStateChanged();
                            }
                        }
                    } else {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
            } else if (this.roomClient != null) {
                if (this.playerID == null) {
                    ReadPlayerID();
                } else {
                    SyncPlayerNames();
                }
                using (var res = this.roomClient.TryRecv()) {
                    if (res.IsOkSome()) {
                        using (var msg = res.UnwrapUnwrap()) {
                            switch (msg.EffectType()) {
                                case 0: { // changes room state
                                    msg.Apply(this.roomClient);
                                    this.roomState.Text = this.roomClient.State().AsString();
                                    break;
                                }
                                case 1: { // sets a player name and changes room state
                                    if (this.coopContextAddr != null) {
                                        APIs.Memory.WriteByteRange(this.coopContextAddr.Value + 0x14 + msg.World() * 0x8, msg.Filename(), "System Bus");
                                    }
                                    msg.Apply(this.roomClient);
                                    this.roomState.Text = this.roomClient.State().AsString();
                                    break;
                                }
                                default: {
                                    Error($"received unknown server message of effect type {msg.EffectType()}");
                                    break;
                                }
                            }
                        }
                    } else if (res.IsErr()) {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
                if (this.playerID != null && this.coopContextAddr != null) {
                    var outgoingKey = APIs.Memory.ReadU32(this.coopContextAddr.Value + 0xc, "System Bus");
                    if (outgoingKey != 0) {
                        var kind = (ushort) APIs.Memory.ReadU16(this.coopContextAddr.Value + 0x10, "System Bus");
                        var player = (byte) APIs.Memory.ReadU16(this.coopContextAddr.Value + 0x12, "System Bus");
                        if (player == this.playerID && kind != 0xca) {
                            //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for myself");
                        } else if (outgoingKey == 0xff05ff) {
                            //Debug($"P{this.playerID}: Found an item {kind} for player {player} sent via network, ignoring");
                        } else {
                            //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for player {player}");
                            this.roomClient.SendItem(outgoingKey, kind, player);
                        }
                        APIs.Memory.WriteU32(this.coopContextAddr.Value + 0xc, 0, "System Bus");
                        APIs.Memory.WriteU16(this.coopContextAddr.Value + 0x10, 0, "System Bus");
                        APIs.Memory.WriteU16(this.coopContextAddr.Value + 0x12, 0, "System Bus");
                    }
                    var stateLogo = APIs.Memory.ReadU32(0x11f200, "RDRAM");
                    var stateMain = APIs.Memory.ReadS8(0x11b92f, "RDRAM");
                    var stateMenu = APIs.Memory.ReadS8(0x1d8dd5, "RDRAM");
                    if (stateLogo != 0x802c_5880 && stateLogo != 0 && stateMain != 1 && stateMain != 2 && stateMenu == 0) {
                        if (APIs.Memory.ReadU16(this.coopContextAddr.Value + 0x8, "System Bus") == 0) {
                            var internalCount = (ushort) APIs.Memory.ReadU16(0x11a5d0 + 0x90, "RDRAM");
                            var externalCount = this.roomClient.ItemQueueLen();
                            if (internalCount < externalCount) {
                                var item = this.roomClient.Item((ushort) internalCount);
                                //Debug($"P{this.playerID}: Received an item {item} from another player");
                                APIs.Memory.WriteU16(this.coopContextAddr.Value + 0x8, item, "System Bus");
                                APIs.Memory.WriteU16(this.coopContextAddr.Value + 0x6, item == 0xca ? (this.playerID == 1 ? 2u : 1) : this.playerID.Value, "System Bus");
                            } else if (internalCount > externalCount) {
                                Error("gap in received items");
                            }
                        }
                    }
                }
            }
        }

        private void JoinRoom(RoomClient client) {
            this.lobbyClient?.Dispose();
            this.lobbyClient = null;
            this.roomClient = client;
            SuspendLayout();
            this.rooms.Visible = false;
            this.password.Visible = false;
            this.createJoinButton.Visible = false;
            this.roomState.Text = client.State().AsString();
            this.roomState.Visible = true;
            ResumeLayout(true);
            ReadPlayerID();
            SyncPlayerNames();
        }

        private void ReadPlayerID() {
            if ((APIs.GameInfo.GetGameInfo()?.Name ?? "Null") == "Null") {
                this.playerID = null;
                this.state.Text = "Please open the ROM…";
            } else {
                var romIdent = APIs.Memory.ReadByteRange(0x20, 0x15, "ROM");
                if (!Enumerable.SequenceEqual(romIdent, new List<byte>(Encoding.UTF8.GetBytes("THE LEGEND OF ZELDA \0")))) {
                    this.playerID = null;
                    this.state.Text = $"Expected OoTR, found {APIs.GameInfo.GetGameInfo()?.Name ?? "Null"}";
                } else {
                    SuspendLayout();
                    //TODO also check OoTR version bytes and error on vanilla OoT
                    this.state.Text = "Waiting for game…";
                    if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x11a5d0 + 0x1c, 6, "RDRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) { // don't set or reset player ID while rom is loaded but not properly initialized
                        var randoContextAddr = APIs.Memory.ReadU32(0x1c6e90 + 0x15d4, "RDRAM");
                        if (randoContextAddr >= 0x8000_0000 && randoContextAddr != 0xffff_ffff) {
                            var newCoopContextAddr = APIs.Memory.ReadU32(randoContextAddr, "System Bus");
                            if (newCoopContextAddr >= 0x8000_0000 && newCoopContextAddr != 0xffff_ffff) {
                                //TODO COOP_VERSION check
                                this.coopContextAddr = newCoopContextAddr;
                                this.playerID = (byte?) APIs.Memory.ReadU8(newCoopContextAddr + 0x4, "System Bus");
                                this.state.Text = $"Connected as world {this.playerID}";
                            } else {
                                this.coopContextAddr = null;
                            }
                        }
                    }
                    ResumeLayout();
                }
            }
            if (this.roomClient != null) {
                using (var res = this.roomClient.SetPlayerID(this.playerID)) {
                    if (!res.IsOk()) {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
            }
        }

        private void SyncPlayerNames() {
            if (this.playerID == null) {
                this.playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
            } else {
                if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x0020 + 0x1c, 6, "SRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) {
                    // get own player name from save file
                    this.playerName = APIs.Memory.ReadByteRange(0x0020 + 0x0024, 8, "SRAM");
                    // always fill player names in co-op context (some player names may go missing seemingly at random while others stay intact, so this has to run every frame)
                    if (this.roomClient != null && this.coopContextAddr != null) {
                        for (var world = 1; world < 256; world++) {
                            APIs.Memory.WriteByteRange(this.coopContextAddr.Value + 0x14 + world * 0x8, this.roomClient.GetPlayerName((byte) world), "System Bus");
                        }
                    }
                } else {
                    // file 1 does not exist, reset player name
                    this.playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
                }
            }
            if (this.roomClient != null) {
                using (var res = this.roomClient.SetPlayerName(this.playerName)) {
                    if (!res.IsOk()) {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
            }
        }

        private void LobbyStateChanged() {
            if (this.rooms.Enabled && this.rooms.Text.Length > 0) {
                this.createJoinButton.Enabled = this.password.Text.Length > 0;
                if (this.rooms.Items.Contains(this.rooms.Text)) {
                    this.createJoinButton.Text = "Join";
                } else {
                    this.createJoinButton.Text = "Create";
                }
            } else {
                this.createJoinButton.Enabled = false;
                this.createJoinButton.Text = "Create/Join";
            }
        }

        private void Error(string msg) {
            this.state.Text = $"error: {msg}";
            if (this.lobbyClient != null) {
                this.lobbyClient.Dispose();
                this.lobbyClient = null;
            }
            if (this.roomClient != null) {
                this.roomClient.Dispose();
                this.roomClient = null;
            }
            HideUI();
        }

        private void HideUI() {
            this.rooms.Visible = false;
            this.password.Visible = false;
            this.createJoinButton.Visible = false;
            this.roomState.Visible = false;
        }

        private void ShowUI() {
            if (this.lobbyClient != null) {
                this.rooms.Visible = true;
                this.password.Visible = true;
                this.createJoinButton.Visible = true;
            }
            if (this.roomClient != null) {
                this.roomState.Visible = true;
            }
        }
    }
}
