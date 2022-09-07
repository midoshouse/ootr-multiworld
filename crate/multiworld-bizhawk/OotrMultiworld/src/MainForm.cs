using System;
using System.Collections.Generic;
using System.Drawing;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Forms;
using BizHawk.Client.Common;
using BizHawk.Client.EmuHawk;

namespace MidosHouse.OotrMultiworld {
    internal class Native {
        [DllImport("multiworld")] internal static extern StringHandle version_string();
        [DllImport("multiworld")] internal static extern BoolResult update_available();
        [DllImport("multiworld")] internal static extern void bool_result_free(IntPtr bool_res);
        [DllImport("multiworld")] internal static extern bool bool_result_is_ok(BoolResult bool_res);
        [DllImport("multiworld")] internal static extern bool bool_result_unwrap(IntPtr bool_res);
        [DllImport("multiworld")] internal static extern StringHandle bool_result_debug_err(IntPtr bool_res);
        [DllImport("multiworld")] internal static extern UnitResult run_updater();
        [DllImport("multiworld")] internal static extern ushort default_port();
        [DllImport("multiworld")] internal static extern ClientResult connect_ipv4(ushort port);
        [DllImport("multiworld")] internal static extern ClientResult connect_ipv6(ushort port);
        [DllImport("multiworld")] internal static extern void client_result_free(IntPtr client_res);
        [DllImport("multiworld")] internal static extern bool client_result_is_ok(ClientResult client_res);
        [DllImport("multiworld")] internal static extern Client client_result_unwrap(IntPtr client_res);
        [DllImport("multiworld")] internal static extern void client_set_error(Client client, OwnedStringHandle msg);
        [DllImport("multiworld")] internal static extern byte client_session_state(Client client);
        [DllImport("multiworld")] internal static extern StringHandle client_debug_err(Client client);
        [DllImport("multiworld")] internal static extern bool client_has_wrong_password(Client client);
        [DllImport("multiworld")] internal static extern void client_reset_wrong_password(Client client);
        [DllImport("multiworld")] internal static extern void client_free(IntPtr client);
        [DllImport("multiworld")] internal static extern StringHandle client_result_debug_err(IntPtr client_res);
        [DllImport("multiworld")] internal static extern void string_free(IntPtr s);
        [DllImport("multiworld")] internal static extern ulong client_num_rooms(Client client);
        [DllImport("multiworld")] internal static extern StringHandle client_room_name(Client client, ulong i);
        [DllImport("multiworld")] internal static extern void string_result_free(IntPtr str_res);
        [DllImport("multiworld")] internal static extern bool string_result_is_ok(StringResult str_res);
        [DllImport("multiworld")] internal static extern StringHandle string_result_unwrap(IntPtr str_res);
        [DllImport("multiworld")] internal static extern StringHandle string_result_debug_err(IntPtr str_res);
        [DllImport("multiworld")] internal static extern UnitResult client_room_connect(Client client, OwnedStringHandle room_name, OwnedStringHandle room_password);
        [DllImport("multiworld")] internal static extern UnitResult client_set_player_id(Client client, byte id);
        [DllImport("multiworld")] internal static extern void unit_result_free(IntPtr unit_res);
        [DllImport("multiworld")] internal static extern bool unit_result_is_ok(UnitResult unit_res);
        [DllImport("multiworld")] internal static extern StringHandle unit_result_debug_err(IntPtr unit_res);
        [DllImport("multiworld")] internal static extern UnitResult client_reset_player_id(Client client);
        [DllImport("multiworld")] internal static extern UnitResult client_set_player_name(Client client, IntPtr name);
        [DllImport("multiworld")] internal static extern byte client_num_players(Client client);
        [DllImport("multiworld")] internal static extern StringHandle client_player_state(Client client, byte player_idx);
        [DllImport("multiworld")] internal static extern StringHandle client_other_room_state(Client client);
        [DllImport("multiworld")] internal static extern UnitResult client_kick_player(Client client, byte player_idx);
        [DllImport("multiworld")] internal static extern OptMessageResult client_try_recv_message(Client client, ushort port);
        [DllImport("multiworld")] internal static extern void opt_message_result_free(IntPtr opt_msg_res);
        [DllImport("multiworld")] internal static extern bool opt_message_result_is_ok_some(OptMessageResult opt_msg_res);
        [DllImport("multiworld")] internal static extern ServerMessage opt_message_result_unwrap_unwrap(IntPtr opt_msg_res);
        [DllImport("multiworld")] internal static extern void message_free(IntPtr msg);
        [DllImport("multiworld")] internal static extern bool opt_message_result_is_err(OptMessageResult opt_msg_res);
        [DllImport("multiworld")] internal static extern StringHandle opt_message_result_debug_err(IntPtr opt_msg_res);
        [DllImport("multiworld")] internal static extern UnitResult client_send_item(Client client, uint key, ushort kind, byte target_world);
        [DllImport("multiworld")] internal static extern ushort client_item_queue_len(Client client);
        [DllImport("multiworld")] internal static extern ushort client_item_kind_at_index(Client client, ushort index);
        [DllImport("multiworld")] internal static extern IntPtr client_get_player_name(Client client, byte world);
    }

    internal class BoolResult : SafeHandle {
        internal BoolResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.bool_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOk() => Native.bool_result_is_ok(this);

        internal bool Unwrap() {
            var inner = Native.bool_result_unwrap(this.handle);
            this.handle = IntPtr.Zero; // bool_result_unwrap takes ownership
            return inner;
        }

        internal StringHandle DebugErr() {
            var err = Native.bool_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // bool_result_debug_err takes ownership
            return err;
        }
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

    internal class Client : SafeHandle {
        internal Client() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.client_free(this.handle);
            }
            return true;
        }

        internal void SetError(string msg) {
            using (var msgHandle = new OwnedStringHandle(msg)) {
                Native.client_set_error(this, msgHandle);
            }
        }

        internal byte SessionState() => Native.client_session_state(this);
        internal StringHandle DebugErr() => Native.client_debug_err(this);

        internal bool HasWrongPassword() => Native.client_has_wrong_password(this);
        internal void ResetWrongPassword() => Native.client_reset_wrong_password(this);
        internal ulong NumRooms() => Native.client_num_rooms(this);
        internal StringHandle RoomName(ulong i) => Native.client_room_name(this, i);

        internal UnitResult CreateJoinRoom(string roomName, string password) {
            using (var nameHandle = new OwnedStringHandle(roomName)) {
                using (var passwordHandle = new OwnedStringHandle(password)) {
                    return Native.client_room_connect(this, nameHandle, passwordHandle);
                }
            }
        }

        internal UnitResult SetPlayerID(byte? id) {
            if (id == null) {
                return Native.client_reset_player_id(this);
            } else {
                return Native.client_set_player_id(this, id.Value);
            }
        }

        internal UnitResult SetPlayerName(List<byte> name) {
            var namePtr = Marshal.AllocHGlobal(8);
            Marshal.Copy(name.ToArray(), 0, namePtr, 8);
            var res = Native.client_set_player_name(this, namePtr);
            Marshal.FreeHGlobal(namePtr);
            return res;
        }

        internal List<byte> GetPlayerName(byte world) {
            var name = new byte[8];
            Marshal.Copy(Native.client_get_player_name(this, world), name, 0, 8);
            return name.ToList();
        }

        internal byte NumPlayers() => Native.client_num_players(this);
        internal StringHandle PlayerState(byte player_idx) => Native.client_player_state(this, player_idx);
        internal StringHandle OtherState() => Native.client_other_room_state(this);
        internal OptMessageResult TryRecv(ushort port) => Native.client_try_recv_message(this, port);
        internal UnitResult SendItem(uint key, ushort kind, byte targetWorld) => Native.client_send_item(this, key, kind, targetWorld);
        internal ushort ItemQueueLen() => Native.client_item_queue_len(this);
        internal ushort Item(ushort index) => Native.client_item_kind_at_index(this, index);
        internal UnitResult KickPlayer(byte player_idx) => Native.client_kick_player(this, player_idx);
    }

    internal class ClientResult : SafeHandle {
        internal ClientResult() : base(IntPtr.Zero, true) {}

        public override bool IsInvalid {
            get { return this.handle == IntPtr.Zero; }
        }

        protected override bool ReleaseHandle() {
            if (!this.IsInvalid) {
                Native.client_result_free(this.handle);
            }
            return true;
        }

        internal bool IsOk() => Native.client_result_is_ok(this);

        internal Client Unwrap() {
            var client = Native.client_result_unwrap(this.handle);
            this.handle = IntPtr.Zero; // client_result_unwrap takes ownership
            return client;
        }

        internal StringHandle DebugErr() {
            var err = Native.client_result_debug_err(this.handle);
            this.handle = IntPtr.Zero; // client_result_debug_err takes ownership
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

    [ExternalTool("Mido's House Multiworld", Description = "Play interconnected Ocarina of Time Randomizer seeds")]
    [ExternalToolEmbeddedIcon("MidosHouse.OotrMultiworld.Resources.icon.ico")]
    public sealed class MainForm : ToolFormBase, IExternalToolForm {
        private Label state = new Label();
        private ComboBox rooms = new ComboBox();
        private TextBox password = new TextBox();
        private Button createJoinButton = new Button();
        private Label version = new Label();
        private List<Label> playerStates = new List<Label>();
        private List<Button> kickButtons = new List<Button>();
        private Label otherState = new Label();

        private ushort port = Native.default_port();
        private Client? client;
        private uint? coopContextAddr;
        private byte? playerID;
        private List<byte> playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };

        public ApiContainer? _apiContainer { get; set; }
        private ApiContainer APIs => _apiContainer ?? throw new NullReferenceException();

        public override bool BlocksInputWhenFocused { get; } = false;
        protected override string WindowTitleStatic => "Mido's House Multiworld for BizHawk";

        public override bool AskSaveChanges() => true; //TODO warn before leaving an active game?

        public MainForm() {
            SuspendLayout();
            this.ClientSize = new Size(509, 256);
            this.Icon = new Icon(typeof(MainForm).Assembly.GetManifestResourceStream("MidosHouse.OotrMultiworld.Resources.icon.ico"));

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
            this.rooms.SelectedIndexChanged += (s, e) => {
                if (this.client != null) {
                    this.UpdateLobbyState(this.client, false);
                }
            };
            this.rooms.TextChanged += (s, e) => {
                if (this.client != null) {
                    this.UpdateLobbyState(this.client, false);
                }
            };
            this.Controls.Add(this.rooms);

            this.password.TabIndex = 2;
            this.password.Location = new Point(12, 82);
            this.password.Size = new Size(485, 25);
            this.password.UseSystemPasswordChar = true;
            //TODO (.net 5) add PlaceholderText (“Password”)
            this.password.TextChanged += (s, e) => {
                this.createJoinButton.Enabled = this.rooms.Enabled && this.rooms.Text.Length > 0 && this.password.Text.Length > 0;
            };
            this.password.KeyDown += (s, e) => {
                if (e.KeyCode == Keys.Enter) {
                    e.SuppressKeyPress = true;
                    CreateJoinRoom();
                }
            };
            this.Controls.Add(this.password);

            this.createJoinButton.TabIndex = 3;
            this.createJoinButton.Location = new Point(11, 119);
            this.createJoinButton.AutoSize = true;
            this.createJoinButton.Text = "Create/Join";
            this.createJoinButton.Enabled = false;
            this.createJoinButton.Click += (s, e) => {
                CreateJoinRoom();
            };
            this.Controls.Add(this.createJoinButton);

            this.version.TabIndex = 4;
            this.version.Location = new Point(162, 119);
            this.version.AutoSize = false;
            this.version.Size = new Size(335, 25);
            this.version.TextAlign = ContentAlignment.MiddleRight;
            using (var versionString = Native.version_string()) {
                this.version.Text = $"v{versionString.AsString()}";
            }
            this.Controls.Add(this.version);

            this.otherState.TabIndex = 4;
            this.otherState.Location = new Point(12, 42);
            this.otherState.AutoSize = true;
            this.otherState.Visible = false;
            this.Controls.Add(this.otherState);

            ResumeLayout(true);
        }

        private void CreateJoinRoom() {
            if (this.client != null) {
                using (var res = this.client.CreateJoinRoom(this.rooms.Text, this.password.Text)) {
                    if (!res.IsOk()) {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
            }
        }

        public override void Restart() {
            APIs.Memory.SetBigEndian(true);
            if ((APIs.GameInfo.GetGameInfo()?.Name ?? "Null") == "Null") {
                this.state.Text = "Please open the ROM…";
                HideLobbyUI();
                HideRoomUI();
                return;
            }
            if (this.client == null) {
                this.state.Text = "Checking for updates…";
                using (var update_available_res = Native.update_available()) {
                    if (update_available_res.IsOk()) {
                        if (update_available_res.Unwrap()) {
                            this.state.Text = "An update is available";
                            using (var run_updater_res = Native.run_updater()) {
                                if (!run_updater_res.IsOk()) {
                                    this.state.Text = run_updater_res.DebugErr().AsString();
                                    return;
                                }
                            }
                        }
                    } else {
                        this.state.Text = update_available_res.DebugErr().AsString();
                        return;
                    }
                }
                this.state.Text = "Connecting…";
                using (var res6 = Native.connect_ipv6(this.port)) {
                    if (res6.IsOk()) {
                        this.client = res6.Unwrap();
                    } else {
                        using (var res4 = Native.connect_ipv4(this.port)) {
                            if (res4.IsOk()) {
                                this.client = res4.Unwrap();
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
            }
            this.playerID = null;
            if (this.client != null) {
                UpdateUI(this.client);
            }
        }

        public override void UpdateValues(ToolFormUpdateType type) {
            if (type != ToolFormUpdateType.PreFrame && type != ToolFormUpdateType.FastPreFrame) {
                return;
            }
            if ((APIs.GameInfo.GetGameInfo()?.Name ?? "Null") == "Null") {
                return;
            }
            if (this.client != null) {
                ReceiveMessage(this.client);
                if (this.client.SessionState() == 4) { // Room
                    if (this.playerID == null) {
                        ReadPlayerID();
                    } else {
                        SyncPlayerNames();
                        if (this.coopContextAddr != null) {
                            SendItem(this.client, this.coopContextAddr.Value);
                            ReceiveItem(this.client, this.coopContextAddr.Value, this.playerID.Value);
                        }
                    }
                }
            }
        }

        private void ReceiveMessage(Client client) {
            using (var res = client.TryRecv(this.port)) {
                if (res.IsOkSome()) {
                    using (var msg = res.UnwrapUnwrap()) {
                        UpdateUI(client);
                    }
                } else if (res.IsErr()) {
                    using (var err = res.DebugErr()) {
                        Error(err.AsString());
                    }
                }
            }
        }

        private void SendItem(Client client, uint coopContextAddr) {
            var outgoingKey = APIs.Memory.ReadU32(coopContextAddr + 0xc, "System Bus");
            if (outgoingKey != 0) {
                var kind = (ushort) APIs.Memory.ReadU16(coopContextAddr + 0x10, "System Bus");
                var player = (byte) APIs.Memory.ReadU16(coopContextAddr + 0x12, "System Bus");
                if (player == this.playerID && kind != 0xca) {
                    //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for myself");
                } else if (outgoingKey == 0xff05ff) {
                    //Debug($"P{this.playerID}: Found an item {kind} for player {player} sent via network, ignoring");
                } else {
                    //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for player {player}");
                    client.SendItem(outgoingKey, kind, player);
                }
                APIs.Memory.WriteU32(coopContextAddr + 0xc, 0, "System Bus");
                APIs.Memory.WriteU16(coopContextAddr + 0x10, 0, "System Bus");
                APIs.Memory.WriteU16(coopContextAddr + 0x12, 0, "System Bus");
            }
        }

        private void ReceiveItem(Client client, uint coopContextAddr, byte playerID) {
            var stateLogo = APIs.Memory.ReadU32(0x11f200, "RDRAM");
            var stateMain = APIs.Memory.ReadS8(0x11b92f, "RDRAM");
            var stateMenu = APIs.Memory.ReadS8(0x1d8dd5, "RDRAM");
            if (stateLogo != 0x802c_5880 && stateLogo != 0 && stateMain != 1 && stateMain != 2 && stateMenu == 0) {
                if (APIs.Memory.ReadU16(coopContextAddr + 0x8, "System Bus") == 0) {
                    var internalCount = (ushort) APIs.Memory.ReadU16(0x11a5d0 + 0x90, "RDRAM");
                    var externalCount = client.ItemQueueLen();
                    if (internalCount < externalCount) {
                        var item = client.Item((ushort) internalCount);
                        //Debug($"P{playerID}: Received an item {item} from another player");
                        APIs.Memory.WriteU16(coopContextAddr + 0x8, item, "System Bus");
                        APIs.Memory.WriteU16(coopContextAddr + 0x6, item == 0xca ? (playerID == 1 ? 2u : 1) : playerID, "System Bus");
                    } else if (internalCount > externalCount) {
                        // warning: gap in received items
                    }
                }
            }
        }

        private void UpdateUI(Client client) {
            switch (client.SessionState()) {
                case 0: { // Error
                    using (var err = client.DebugErr()) {
                        this.state.Text = $"error: {err.AsString()}";
                        HideLobbyUI();
                        HideRoomUI();
                    }
                    break;
                }
                case 1: { // Init
                    this.state.Text = "Loading room list…";
                    HideLobbyUI();
                    HideRoomUI();
                    break;
                }
                case 2: { // InitAutoRejoin
                    this.state.Text = "Reconnecting to room…";
                    HideLobbyUI();
                    HideRoomUI();
                    break;
                }
                case 3: { // Lobby
                    this.UpdateLobbyState(client, true);
                    break;
                }
                case 4: { // Room
                    this.UpdateRoomState(client);
                    break;
                }
                case 5: { // Closed
                    this.state.Text = "You have been disconnected. Reopen the tool to reconnect."; //TODO reconnect button
                    HideLobbyUI();
                    HideRoomUI();
                    break;
                }
                default: {
                    Error("received unknown session state type");
                    break;
                }
            }
        }

        private void UpdateLobbyState(Client client, bool updateRoomList) {
            if (client.HasWrongPassword()) {
                this.DialogController.ShowMessageBox(this, "wrong password", null, EMsgBoxIcon.Error);
                client.ResetWrongPassword();
                this.password.Text = "";
            }
            var numRooms = client.NumRooms();
            this.state.Text = "Join or create a room:";
            SuspendLayout();
            HideRoomUI();
            this.rooms.Visible = true;
            this.password.Visible = true;
            this.createJoinButton.Visible = true;
            this.version.Visible = true;
            if (updateRoomList) {
                this.rooms.SelectedItem = null;
                this.rooms.Items.Clear();
                for (ulong i = 0; i < numRooms; i++) {
                    this.rooms.Items.Add(client.RoomName(i).AsString());
                }
            }
            this.rooms.Enabled = true;
            if (this.rooms.Text.Length > 0) {
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
            ResumeLayout(true);
        }

        private void UpdateRoomState(Client client) {
            SuspendLayout();
            HideLobbyUI();
            var num_players = client.NumPlayers();
            for (byte player_idx = 0; player_idx < num_players; player_idx++) {
                if (player_idx >= this.playerStates.Count) {
                    var playerState = new Label();
                    playerState.TabIndex = 2 * player_idx + 4;
                    playerState.Location = new Point(92, 40 * player_idx + 42);
                    playerState.AutoSize = true;
                    playerState.Visible = true;
                    this.Controls.Add(playerState);
                    this.playerStates.Add(playerState);

                    var kickButton = new Button();
                    kickButton.TabIndex = 2 * player_idx + 5;
                    kickButton.Location = new Point(12, 40 * player_idx + 42);
                    kickButton.AutoSize = true;
                    kickButton.Visible = true;
                    kickButton.Text = "Kick";
                    kickButton.Enabled = true;
                    var closurePlayerIdx = player_idx;
                    kickButton.Click += (s, e) => {
                        using (var res = client.KickPlayer(closurePlayerIdx)) {
                            if (!res.IsOk()) {
                                using (var err = res.DebugErr()) {
                                    Error(err.AsString());
                                }
                            }
                        }
                    };
                    this.Controls.Add(kickButton);
                    this.kickButtons.Add(kickButton);
                }
                this.playerStates[player_idx].Text = client.PlayerState(player_idx).AsString();
            }
            this.otherState.TabIndex = 2 * num_players + 4;
            this.otherState.Location = new Point(12, 40 * num_players + 42);
            this.otherState.Visible = true;
            this.otherState.Text = client.OtherState().AsString();
            if (num_players < this.playerStates.Count) {
                for (var player_idx = num_players; player_idx < this.playerStates.Count; player_idx++) {
                    this.Controls.Remove(this.playerStates[player_idx]);
                    this.Controls.Remove(this.kickButtons[player_idx]);
                }
                this.playerStates.RemoveRange(num_players, this.playerStates.Count - num_players);
                this.kickButtons.RemoveRange(num_players, this.kickButtons.Count - num_players);
            }
            ResumeLayout();
        }

        private void ReadPlayerID() {
            var oldPlayerID = this.playerID;
            if ((APIs.GameInfo.GetGameInfo()?.Name ?? "Null") == "Null") {
                this.playerID = null;
                this.state.Text = "Please open the ROM…";
                PerformLayout();
            } else {
                var romIdent = APIs.Memory.ReadByteRange(0x20, 0x15, "ROM");
                if (!Enumerable.SequenceEqual(romIdent, new List<byte>(Encoding.UTF8.GetBytes("THE LEGEND OF ZELDA \0")))) {
                    this.playerID = null;
                    this.state.Text = $"Expected OoTR, found {APIs.GameInfo.GetGameInfo()?.Name ?? "Null"}";
                    PerformLayout();
                } else {
                    //TODO also check OoTR version bytes and error on vanilla OoT
                    var newText = "Waiting for game…";
                    if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x11a5d0 + 0x1c, 6, "RDRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) { // don't set or reset player ID while rom is loaded but not properly initialized
                        var randoContextAddr = APIs.Memory.ReadU32(0x1c6e90 + 0x15d4, "RDRAM");
                        if (randoContextAddr >= 0x8000_0000 && randoContextAddr != 0xffff_ffff) {
                            var newCoopContextAddr = APIs.Memory.ReadU32(randoContextAddr, "System Bus");
                            if (newCoopContextAddr >= 0x8000_0000 && newCoopContextAddr != 0xffff_ffff) {
                                //TODO COOP_VERSION check
                                this.coopContextAddr = newCoopContextAddr;
                                this.playerID = (byte?) APIs.Memory.ReadU8(newCoopContextAddr + 0x4, "System Bus");
                                newText = $"Connected as world {this.playerID}";
                            } else {
                                this.coopContextAddr = null;
                            }
                        }
                    }
                    if (this.state.Text != newText) {
                        this.state.Text = newText;
                        PerformLayout();
                    }
                }
            }
            if (this.client != null && this.playerID != oldPlayerID) {
                using (var res = this.client.SetPlayerID(this.playerID)) {
                    if (res.IsOk()) {
                        UpdateUI(this.client);
                    } else {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
            }
        }

        private void SyncPlayerNames() {
            var oldPlayerName = this.playerName;
            if (this.playerID == null) {
                this.playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
            } else {
                if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x0020 + 0x1c, 6, "SRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) {
                    // get own player name from save file
                    this.playerName = APIs.Memory.ReadByteRange(0x0020 + 0x0024, 8, "SRAM");
                    // always fill player names in co-op context (some player names may go missing seemingly at random while others stay intact, so this has to run every frame)
                    if (this.client != null && this.client.SessionState() == 4 /* Room */ && this.coopContextAddr != null) {
                        for (var world = 1; world < 256; world++) {
                            APIs.Memory.WriteByteRange(this.coopContextAddr.Value + 0x14 + world * 0x8, this.client.GetPlayerName((byte) world), "System Bus");
                        }
                    }
                } else {
                    // file 1 does not exist, reset player name
                    this.playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
                }
            }
            if (this.client != null && !Enumerable.SequenceEqual(this.playerName, oldPlayerName)) {
                using (var res = this.client.SetPlayerName(this.playerName)) {
                    if (res.IsOk()) {
                        UpdateUI(this.client);
                    } else {
                        using (var err = res.DebugErr()) {
                            Error(err.AsString());
                        }
                    }
                }
            }
        }

        private void Error(string msg) {
            if (this.client == null) {
                this.state.Text = $"error: {msg}";
                HideLobbyUI();
                HideRoomUI();
            } else {
                this.client.SetError(msg);
                UpdateUI(this.client);
            }
        }

        private void HideLobbyUI() {
            this.rooms.Visible = false;
            this.password.Visible = false;
            this.createJoinButton.Visible = false;
            this.version.Visible = false;
        }

        private void HideRoomUI() {
            for (var player_idx = 0; player_idx < this.playerStates.Count; player_idx++) {
                this.playerStates[player_idx].Visible = false;
                this.kickButtons[player_idx].Visible = false;
            }
            this.otherState.Visible = false;
        }
    }
}
