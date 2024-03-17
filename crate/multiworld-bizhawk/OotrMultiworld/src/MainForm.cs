using System;
using System.Collections.Generic;
using System.Drawing;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Forms;
using BizHawk.Client.Common;
using BizHawk.Client.EmuHawk;
using BizHawk.Common;

namespace MidosHouse.OotrMultiworld;

enum OptHintArea : byte {
    Unknown,
    Root,
    HyruleField,
    LonLonRanch,
    Market,
    TempleOfTime,
    HyruleCastle,
    OutsideGanonsCastle,
    InsideGanonsCastle,
    KokiriForest,
    DekuTree,
    LostWoods,
    SacredForestMeadow,
    ForestTemple,
    DeathMountainTrail,
    DodongosCavern,
    GoronCity,
    DeathMountainCrater,
    FireTemple,
    ZoraRiver,
    ZorasDomain,
    ZorasFountain,
    JabuJabusBelly,
    IceCavern,
    LakeHylia,
    WaterTemple,
    KakarikoVillage,
    BottomOfTheWell,
    Graveyard,
    ShadowTemple,
    GerudoValley,
    GerudoFortress,
    ThievesHideout,
    GerudoTrainingGround,
    HauntedWasteland,
    DesertColossus,
    SpiritTemple,
}

internal class Native {
    [DllImport("multiworld")] internal static extern void log(OwnedStringHandle msg);
    [DllImport("multiworld")] internal static extern void log_init();
    [DllImport("multiworld")] internal static extern void error_free(IntPtr error);
    [DllImport("multiworld")] internal static extern Error error_from_string(OwnedStringHandle text);
    [DllImport("multiworld")] internal static extern StringHandle error_debug(Error error);
    [DllImport("multiworld")] internal static extern StringHandle error_display(Error error);
    [DllImport("multiworld")] internal static extern ClientResult open_gui(OwnedStringHandle version);
    [DllImport("multiworld")] internal static extern void client_result_free(IntPtr client_res);
    [DllImport("multiworld")] internal static extern bool client_result_is_ok(ClientResult client_res);
    [DllImport("multiworld")] internal static extern Client client_result_unwrap(IntPtr client_res);
    [DllImport("multiworld")] internal static extern Error client_result_unwrap_err(IntPtr client_res);
    [DllImport("multiworld")] internal static extern void client_free(IntPtr client);
    [DllImport("multiworld")] internal static extern void string_free(IntPtr s);
    [DllImport("multiworld")] internal static extern UnitResult client_set_player_id(Client client, byte id);
    [DllImport("multiworld")] internal static extern void unit_result_free(IntPtr unit_res);
    [DllImport("multiworld")] internal static extern bool unit_result_is_ok(UnitResult unit_res);
    [DllImport("multiworld")] internal static extern Error unit_result_unwrap_err(IntPtr unit_res);
    [DllImport("multiworld")] internal static extern UnitResult client_reset_player_id(Client client);
    [DllImport("multiworld")] internal static extern UnitResult client_set_player_name(Client client, IntPtr name);
    [DllImport("multiworld")] internal static extern UnitResult client_set_file_hash(Client client, IntPtr hash);
    [DllImport("multiworld")] internal static extern UnitResult client_set_save_data(Client client, IntPtr save);
    [DllImport("multiworld")] internal static extern OptMessageResult client_try_recv_message(Client client);
    [DllImport("multiworld")] internal static extern void opt_message_result_free(IntPtr opt_msg_res);
    [DllImport("multiworld")] internal static extern sbyte opt_message_result_kind(OptMessageResult opt_msg_res);
    [DllImport("multiworld")] internal static extern ushort opt_message_result_item_queue_len(OptMessageResult opt_msg_res);
    [DllImport("multiworld")] internal static extern ushort opt_message_result_item_kind_at_index(OptMessageResult opt_msg_res, ushort index);
    [DllImport("multiworld")] internal static extern byte opt_message_result_world_id(OptMessageResult opt_msg_res);
    [DllImport("multiworld")] internal static extern uint opt_message_result_progressive_items(OptMessageResult opt_msg_res);
    [DllImport("multiworld")] internal static extern IntPtr opt_message_result_filename(OptMessageResult opt_msg_res);
    [DllImport("multiworld")] internal static extern Error opt_message_result_unwrap_err(IntPtr opt_msg_res);
    [DllImport("multiworld")] internal static extern UnitResult client_send_item(Client client, ulong key, ushort kind, byte target_world);
    [DllImport("multiworld")] internal static extern UnitResult client_send_dungeon_reward_info(byte emerald_world, OptHintArea emerald_area, byte ruby_world, OptHintArea ruby_area, byte sapphire_world, OptHintArea sapphire_area, byte light_world, OptHintArea light_area, byte forest_world, OptHintArea forest_area, byte fire_world, OptHintArea fire_area, byte water_world, OptHintArea water_area, byte shadow_world, OptHintArea shadow_area, byte spirit_world, OptHintArea spirit_area, Client client);
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

internal class ClientResult : SafeHandle {
    internal ClientResult() : base(IntPtr.Zero, true) {}

    static internal ClientResult open() {
        using (var versionHandle = new OwnedStringHandle(VersionInfo.MainVersion)) {
            return Native.open_gui(versionHandle);
        }
    }

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

    internal Error UnwrapErr() {
        var err = Native.client_result_unwrap_err(this.handle);
        this.handle = IntPtr.Zero; // client_result_unwrap_err takes ownership
        return err;
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

    internal void SetPlayerID(byte? id) {
        if (id == null) {
            Native.client_reset_player_id(this);
        } else {
            Native.client_set_player_id(this, id.Value);
        }
    }

    internal void SetPlayerName(IReadOnlyList<byte> name) {
        var namePtr = Marshal.AllocHGlobal(8);
        Marshal.Copy(name.ToArray(), 0, namePtr, 8);
        Native.client_set_player_name(this, namePtr);
        Marshal.FreeHGlobal(namePtr);
    }

    internal void SendSaveData(IReadOnlyList<byte> saveData) {
        var savePtr = Marshal.AllocHGlobal(0x1450);
        Marshal.Copy(saveData.ToArray(), 0, savePtr, 0x1450);
        Native.client_set_save_data(this, savePtr);
        Marshal.FreeHGlobal(savePtr);
    }

    internal void SendFileHash(IReadOnlyList<byte> fileHash) {
        var hashPtr = Marshal.AllocHGlobal(5);
        Marshal.Copy(fileHash.ToArray(), 0, hashPtr, 5);
        Native.client_set_file_hash(this, hashPtr);
        Marshal.FreeHGlobal(hashPtr);
    }

    internal OptMessageResult TryRecv() => Native.client_try_recv_message(this);
    internal void SendItem(ulong key, ushort kind, byte targetWorld) => Native.client_send_item(this, key, kind, targetWorld);
}

internal class Error : SafeHandle {
    internal Error() : base(IntPtr.Zero, true) {}

    static internal Error from_string(string text) {
        using (var textHandle = new OwnedStringHandle(text)) {
            return Native.error_from_string(textHandle);
        }
    }

    public override bool IsInvalid {
        get { return this.handle == IntPtr.Zero; }
    }

    protected override bool ReleaseHandle() {
        if (!this.IsInvalid) {
            Native.error_free(this.handle);
        }
        return true;
    }

    internal StringHandle Debug() => Native.error_debug(this);
    internal StringHandle Display() => Native.error_display(this);
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

    internal sbyte Kind() => Native.opt_message_result_kind(this);
    internal ushort ItemQueueLen() => Native.opt_message_result_item_queue_len(this);
    internal ushort ItemAtIndex(ushort index) => Native.opt_message_result_item_kind_at_index(this, index);
    internal byte WorldId() => Native.opt_message_result_world_id(this);
    internal uint ProgressiveItems() => Native.opt_message_result_progressive_items(this);

    internal List<byte> Filename() {
        var name = new byte[8];
        Marshal.Copy(Native.opt_message_result_filename(this), name, 0, 8);
        return name.ToList();
    }

    internal Error UnwrapErr() {
        var err = Native.opt_message_result_unwrap_err(this.handle);
        this.handle = IntPtr.Zero; // opt_msg_result_unwrap_err takes ownership
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

    internal Error UnwrapErr() {
        var err = Native.unit_result_unwrap_err(this.handle);
        this.handle = IntPtr.Zero; // unit_result_unwrap_err takes ownership
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
    private readonly List<byte> REWARD_ROWS = new List<byte> { 0, 1, 2, 8, 3, 4, 5, 7, 6 };

    private Client? client;
    private List<byte> fileHash = new List<byte> { 0xff, 0xff, 0xff, 0xff, 0xff };
    private byte? playerID;
    private List<byte> playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
    private List<List<byte>> playerNames = new List<List<byte>> {
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x05 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x01, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x02, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x01 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x03, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x07 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x04, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x05, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x03 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x06, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x07, 0x09 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x08, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x05 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0x09, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x00, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x01 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x01, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x07 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x02, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x03, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x03 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x04, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x05, 0x09 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x06, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x05 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x07, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x08, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x01 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x01, 0x09, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x07 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x00, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x01, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x03 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x02, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x03, 0x09 },
        new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x05 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x06 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x07 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x08 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x04, 0x09 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x05, 0x00 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x05, 0x01 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x05, 0x02 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x05, 0x03 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x05, 0x04 }, new List<byte> { 0xba, 0xd0, 0xc5, 0xdd, 0xd6, 0x02, 0x05, 0x05 },
    };
    private bool progressiveItemsEnable = false;
    private List<uint> progressiveItems = new List<uint>(Enumerable.Repeat(0u, 256));
    private List<ushort> itemQueue = new List<ushort>();
    private bool normalGameplay = false;
    private bool potsanity3 = false;

    public ApiContainer? _apiContainer { get; set; }
    private ApiContainer APIs => _apiContainer ?? throw new NullReferenceException();

    public override bool BlocksInputWhenFocused { get; } = false;
    protected override string WindowTitleStatic => "Mido's House Multiworld for BizHawk";

    public override bool AskSaveChanges() => true;

    public MainForm() {
        Native.log_init();
        this.ShowInTaskbar = false;
        this.WindowState = FormWindowState.Minimized;
        this.FormBorderStyle = FormBorderStyle.None;
        this.Size = new Size(0, 0);
        this.Icon = new Icon(typeof(MainForm).Assembly.GetManifestResourceStream("MidosHouse.OotrMultiworld.Resources.icon.ico"));
    }

    public override void Restart() {
        APIs.Memory.SetBigEndian(true);
        if (this.client == null) {
            using (var res = ClientResult.open()) {
                if (res.IsOk()) {
                    this.client = res.Unwrap();
                } else {
                    using (var err = res.UnwrapErr()) {
                        SetError(err);
                    }
                }
            }
        }
        this.playerID = null;
        if (this.client != null) {
            this.client.SetPlayerID(this.playerID);
        }
    }

    public override void UpdateValues(ToolFormUpdateType type) {
        if (type != ToolFormUpdateType.PreFrame && type != ToolFormUpdateType.FastPreFrame) {
            return;
        }
        if ((APIs.Emulation.GetGameInfo()?.Name ?? "Null") == "Null") {
            this.normalGameplay = false;
            return;
        }
        if (this.client != null) {
            ReceiveMessage(this.client);
            var coopContextAddr = ReadPlayerID();
            SyncPlayerNames(coopContextAddr);
            if (coopContextAddr != null) {
                if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x11a5d0 + 0x1c, 6, "RDRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) { // don't read save data while rom is loaded but not properly initialized
                    var randoContextAddr = APIs.Memory.ReadU32(0x1c6e90 + 0x15d4, "RDRAM");
                    if (randoContextAddr >= 0x8000_0000 && randoContextAddr != 0xffff_ffff) {
                        var newCoopContextAddr = APIs.Memory.ReadU32(randoContextAddr, "System Bus");
                        if (newCoopContextAddr >= 0x8000_0000 && newCoopContextAddr != 0xffff_ffff) {
                            if (APIs.Memory.ReadU32(0x11a5d0 + 0x135c, "RDRAM") == 0) { // game mode == gameplay
                                SendDungeonRewardLocationInfo(this.client, this.playerID.Value, APIs.Memory.ReadU32(randoContextAddr + 0x4, "System Bus"), APIs.Memory.ReadU32(randoContextAddr + 0xc, "System Bus"));
                                if (!this.normalGameplay) {
                                    this.client.SendSaveData(APIs.Memory.ReadByteRange(0x11a5d0, 0x1450, "RDRAM"));
                                    this.normalGameplay = true;
                                }
                            } else {
                                this.normalGameplay = false;
                            }
                        } else {
                            this.normalGameplay = false;
                        }
                    } else {
                        this.normalGameplay = false;
                    }
                } else {
                    this.normalGameplay = false;
                }

                SendItem(this.client, coopContextAddr.Value);
                ReceiveItem(this.client, coopContextAddr.Value, this.playerID.Value);
            } else {
                this.normalGameplay = false;
            }
        } else {
            this.normalGameplay = false;
        }
    }

    private void ReceiveMessage(Client client) {
        using (var msg = client.TryRecv()) {
            switch (msg.Kind()) {
                case -1: // no message
                    break;
                case -2: // error
                    using (var err = msg.UnwrapErr()) {
                        SetError(err);
                    }
                    break;
                case -3: // GUI closed
                    if (this.client != null) {
                        this.client.Dispose();
                        this.client = null;
                    }
                    this.Close();
                    break;
                case 0: // ServerMessage::ItemQueue
                    this.itemQueue.Clear();
                    var len = msg.ItemQueueLen();
                    for (ushort i = 0; i < len; i++) {
                        this.itemQueue.Add(msg.ItemAtIndex(i));
                    }
                    break;
                case 1: // ServerMessage::GetItem
                    this.itemQueue.Add(msg.ItemAtIndex(0));
                    break;
                case 2: // ServerMessage::PlayerName
                    this.playerNames[msg.WorldId()] = msg.Filename();
                    break;
                case 3: // ServerMessage::ProgressiveItems
                    this.progressiveItems[msg.WorldId()] = msg.ProgressiveItems();
                    break;
                default:
                    using (var error = Error.from_string("BizHawk frontend received unknown command from client")) {
                        SetError(error);
                    }
                    return;
            }
        }
    }

    private void SendItem(Client client, uint coopContextAddr) {
        ulong outgoingKey;
        if (this.potsanity3) {
            outgoingKey = APIs.Memory.ReadU32(coopContextAddr + 0x0c1c, "System Bus") << 32;
            outgoingKey |= APIs.Memory.ReadU32(coopContextAddr + 0x0c20, "System Bus");
        } else {
            outgoingKey = APIs.Memory.ReadU32(coopContextAddr + 0xc, "System Bus");
        }
        if (outgoingKey != 0) {
            var kind = (ushort) APIs.Memory.ReadU16(coopContextAddr + 0x10, "System Bus");
            var player = (byte) APIs.Memory.ReadU16(coopContextAddr + 0x12, "System Bus");
            if (outgoingKey == 0xff05ff) {
                //Debug($"P{this.playerID}: Found an item {kind} for player {player} sent via network, ignoring");
            } else {
                //Debug($"P{this.playerID}: Found {outgoingKey}, an item {kind} for player {player}");
                client.SendItem(outgoingKey, kind, player);
            }
            APIs.Memory.WriteU16(coopContextAddr + 0x10, 0, "System Bus");
            APIs.Memory.WriteU16(coopContextAddr + 0x12, 0, "System Bus");
            if (this.potsanity3) {
                APIs.Memory.WriteU32(coopContextAddr + 0x0c1c, 0, "System Bus");
                APIs.Memory.WriteU32(coopContextAddr + 0x0c20, 0, "System Bus");
            } else {
                APIs.Memory.WriteU32(coopContextAddr + 0xc, 0, "System Bus");
            }
        }
    }

    private void ReceiveItem(Client client, uint coopContextAddr, byte playerID) {
        var stateLogo = APIs.Memory.ReadU32(0x11f200, "RDRAM");
        var stateMain = APIs.Memory.ReadS8(0x11b92f, "RDRAM");
        var stateMenu = APIs.Memory.ReadS8(0x1d8dd5, "RDRAM");
        var currentScene = APIs.Memory.ReadU8(0x1c8545, "RDRAM");
        // The following conditional will be made redundant by https://github.com/TestRunnerSRL/OoT-Randomizer/pull/1867. Keep it for back-compat for now.
        if (
            stateLogo != 0x802c_5880 && stateLogo != 0 && stateMain != 1 && stateMain != 2 && stateMenu == 0 && (
                (currentScene < 0x2c || currentScene > 0x33) && currentScene != 0x42 && currentScene != 0x4b // don't receive items in shops to avoid a softlock when buying an item at the same time as receiving one
            )
        ) {
            if (APIs.Memory.ReadU16(coopContextAddr + 0x8, "System Bus") == 0) {
                var internalCount = (ushort) APIs.Memory.ReadU16(0x11a5d0 + 0x90, "RDRAM");
                var externalCount = this.itemQueue.Count;
                if (internalCount < externalCount) {
                    var item = this.itemQueue[internalCount];
                    //Debug($"P{playerID}: Received an item {item} from player {playerID}");
                    APIs.Memory.WriteU16(coopContextAddr + 0x8, item, "System Bus");
                    APIs.Memory.WriteU16(coopContextAddr + 0x6, item == 0x00ca ? (playerID == 1 ? 2u : 1) : playerID, "System Bus");
                } else if (internalCount > externalCount) {
                    //Debug($"warning: gap in received items: external count is {externalCount} but internal count is {internalCount}");
                }
            }
        }
    }

    private uint? ReadPlayerID() {
        var oldPlayerID = this.playerID;
        if ((APIs.Emulation.GetGameInfo()?.Name ?? "Null") == "Null") {
            this.playerID = null;
            //TODO send state to GUI? ("Please open the ROM…")
        } else {
            var romIdent = APIs.Memory.ReadByteRange(0x20, 0x15, "ROM");
            if (!Enumerable.SequenceEqual(romIdent, new List<byte>(Encoding.UTF8.GetBytes("THE LEGEND OF ZELDA \0")))) {
                this.playerID = null;
                //TODO send state to GUI? ($"Expected OoTR, found {APIs.Emulation.GetGameInfo()?.Name ?? "Null"}")
            } else {
                //TODO also check OoTR version bytes and error on vanilla OoT
                if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x11a5d0 + 0x1c, 6, "RDRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) { // don't set or reset player ID while rom is loaded but not properly initialized
                    var randoContextAddr = APIs.Memory.ReadU32(0x1c6e90 + 0x15d4, "RDRAM");
                    if (randoContextAddr >= 0x8000_0000 && randoContextAddr != 0xffff_ffff) {
                        var newCoopContextAddr = APIs.Memory.ReadU32(randoContextAddr, "System Bus");
                        if (newCoopContextAddr >= 0x8000_0000 && newCoopContextAddr != 0xffff_ffff) {
                            var coopContextVersion = APIs.Memory.ReadU32(newCoopContextAddr, "System Bus");
                            if (coopContextVersion < 2) {
                                using (var error = Error.from_string("randomizer version too old (version 5.1.4 or higher required)")) {
                                    SetError(error);
                                }
                                return null;
                            }
                            if (coopContextVersion > 7) {
                                using (var error = Error.from_string("randomizer version too new (please tell Fenhl that Mido's House Multiworld needs to be updated)")) {
                                    SetError(error);
                                }
                                return null;
                            }
                            if (coopContextVersion == 7) {
                                var branchID = APIs.Memory.ReadU8(0x1c, "ROM");
                                if (branchID == 0x45 || branchID == 0xfe) {
                                    // on Dev-Rob and dev-fenhl, version 7 is https://github.com/OoTRandomizer/OoT-Randomizer/pull/2069
                                    this.potsanity3 = true;
                                } else {
                                    using (var error = Error.from_string("randomizer version too new (please tell Fenhl that Mido's House Multiworld needs to be updated)")) {
                                        SetError(error);
                                    }
                                }
                            } else {
                                this.potsanity3 = false;
                            }
                            if (coopContextVersion >= 3) {
                                APIs.Memory.WriteU8(newCoopContextAddr + 0x000a, 1, "System Bus"); // enable MW_SEND_OWN_ITEMS for server-side tracking
                            }
                            if (coopContextVersion >= 4) {
                                var newFileHash = APIs.Memory.ReadByteRange(newCoopContextAddr + 0x0814, 5, "System Bus");
                                if (this.client != null && !Enumerable.SequenceEqual(this.fileHash, newFileHash)) {
                                    this.client.SendFileHash(newFileHash);
                                    this.fileHash = new List<byte>(newFileHash);
                                }
                            }
                            if (coopContextVersion >= 5) {
                                this.progressiveItemsEnable = true;
                                APIs.Memory.WriteU8(newCoopContextAddr + 0x000b, 1); // MW_PROGRESSIVE_ITEMS_ENABLE
                            } else {
                                this.progressiveItemsEnable = false;
                            }
                            this.playerID = (byte?) APIs.Memory.ReadU8(newCoopContextAddr + 0x4, "System Bus");
                            if (this.client != null && this.playerID != oldPlayerID) {
                                this.client.SetPlayerID(this.playerID);
                            }
                            return newCoopContextAddr;
                        }
                    }
                }
            }
        }
        if (this.client != null && this.playerID != oldPlayerID) {
            this.client.SetPlayerID(this.playerID);
        }
        return null;
    }

    private void SyncPlayerNames(uint? coopContextAddr) {
        var oldPlayerName = this.playerName;
        if (this.playerID == null) {
            this.playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
        } else {
            if (Enumerable.SequenceEqual(APIs.Memory.ReadByteRange(0x0020 + 0x1c, 6, "SRAM"), new List<byte>(Encoding.UTF8.GetBytes("ZELDAZ")))) {
                // get own player name from save file
                this.playerName = new List<byte>(APIs.Memory.ReadByteRange(0x0020 + 0x0024, 8, "SRAM"));
                // always fill player names in co-op context (some player names may go missing seemingly at random while others stay intact, so this has to run every frame)
                if (coopContextAddr != null) {
                    for (var world = 0; world < 256; world++) {
                        APIs.Memory.WriteByteRange(coopContextAddr.Value + 0x14 + world * 0x8, this.playerNames[world], "System Bus");
                        // fill progressive items of other players
                        if (progressiveItemsEnable) {
                            APIs.Memory.WriteU32(coopContextAddr.Value + 0x081c + world * 0x4, this.progressiveItems[world], "System Bus");
                        }
                    }
                }
            } else {
                // file 1 does not exist, reset player name
                this.playerName = new List<byte> { 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf, 0xdf };
            }
        }
        if (this.client != null && !Enumerable.SequenceEqual(this.playerName, oldPlayerName)) {
            this.client.SetPlayerName(this.playerName);
        }
    }

    private OptHintArea HintAreaFromDungeonIdx(byte i) {
        switch (i) {
            case 0: return OptHintArea.DekuTree;
            case 1: return OptHintArea.DodongosCavern;
            case 2: return OptHintArea.JabuJabusBelly;
            case 3: return OptHintArea.ForestTemple;
            case 4: return OptHintArea.FireTemple;
            case 5: return OptHintArea.WaterTemple;
            case 6: return OptHintArea.SpiritTemple;
            case 7: return OptHintArea.ShadowTemple;
            case 8: return OptHintArea.BottomOfTheWell;
            case 9: return OptHintArea.IceCavern;
            case 10: return OptHintArea.InsideGanonsCastle;
            case 11: return OptHintArea.GerudoTrainingGround;
            case 12: return OptHintArea.ThievesHideout;
            case 13: return OptHintArea.InsideGanonsCastle;
            default: return OptHintArea.Unknown;
        }
    }

    private OptHintArea HintAreaFromRewardInfo(uint trackerCtxAddr, byte i) {
        var text = System.Text.Encoding.UTF8.GetString(APIs.Memory.ReadByteRange(trackerCtxAddr + 0x54 + 0x17 * i, 0x16, "System Bus").ToArray());
        if (text == "Free                  ") return OptHintArea.Root;
        if (text == "Hyrule Field          ") return OptHintArea.HyruleField;
        if (text == "Lon Lon Ranch         ") return OptHintArea.LonLonRanch;
        if (text == "Market                ") return OptHintArea.Market;
        if (text == "Temple of Time        ") return OptHintArea.TempleOfTime;
        if (text == "Hyrule Castle         ") return OptHintArea.HyruleCastle;
        if (text == "Outside Ganon's Castle") return OptHintArea.OutsideGanonsCastle;
        if (text == "Inside Ganon's Castle ") return OptHintArea.InsideGanonsCastle;
        if (text == "Kokiri Forest         ") return OptHintArea.KokiriForest;
        if (text == "Deku Tree             ") return OptHintArea.DekuTree;
        if (text == "Lost Woods            ") return OptHintArea.LostWoods;
        if (text == "Sacred Forest Meadow  ") return OptHintArea.SacredForestMeadow;
        if (text == "Forest Temple         ") return OptHintArea.ForestTemple;
        if (text == "Death Mountain Trail  ") return OptHintArea.DeathMountainTrail;
        if (text == "Dodongo's Cavern      ") return OptHintArea.DodongosCavern;
        if (text == "Goron City            ") return OptHintArea.GoronCity;
        if (text == "Death Mountain Crater ") return OptHintArea.DeathMountainCrater;
        if (text == "Fire Temple           ") return OptHintArea.FireTemple;
        if (text == "Zora's River          ") return OptHintArea.ZoraRiver;
        if (text == "Zora's Domain         ") return OptHintArea.ZorasDomain;
        if (text == "Zora's Fountain       ") return OptHintArea.ZorasFountain;
        if (text == "Jabu Jabu's Belly     ") return OptHintArea.JabuJabusBelly;
        if (text == "Ice Cavern            ") return OptHintArea.IceCavern;
        if (text == "Lake Hylia            ") return OptHintArea.LakeHylia;
        if (text == "Water Temple          ") return OptHintArea.WaterTemple;
        if (text == "Kakariko Village      ") return OptHintArea.KakarikoVillage;
        if (text == "Bottom of the Well    ") return OptHintArea.BottomOfTheWell;
        if (text == "Graveyard             ") return OptHintArea.Graveyard;
        if (text == "Shadow Temple         ") return OptHintArea.ShadowTemple;
        if (text == "Gerudo Valley         ") return OptHintArea.GerudoValley;
        if (text == "Gerudo's Fortress     ") return OptHintArea.GerudoFortress;
        if (text == "Thieves' Hideout      ") return OptHintArea.ThievesHideout;
        if (text == "Gerudo Training Ground") return OptHintArea.GerudoTrainingGround;
        if (text == "Haunted Wasteland     ") return OptHintArea.HauntedWasteland;
        if (text == "Desert Colossus       ") return OptHintArea.DesertColossus;
        if (text == "Spirit Temple         ") return OptHintArea.SpiritTemple;
        return OptHintArea.Unknown;
    }

    private void SendDungeonRewardLocationInfo(Client client, byte playerID, uint cosmeticsCtxAddr, uint trackerCtxAddr) {
        if (trackerCtxAddr == 0) { return; }
        var trackerCtxVersion = APIs.Memory.ReadU32(trackerCtxAddr, "System Bus");
        if (trackerCtxVersion < 4) { return; } // partial functionality is available in older rando versions, but supporting those is not worth the effort of checking rando version to disambiguate tracker context v3
        // CAN_DRAW_DUNGEON_INFO
        var cfg_dungeon_info_enable = APIs.Memory.ReadU32(trackerCtxAddr + 0x04, "System Bus");
        if (cfg_dungeon_info_enable == 0) { return; }
        var pause_state = APIs.Memory.ReadU16(0x1d8c00 + 0x01d4, "RDRAM");
        if (pause_state != 6) { return; }
        var pause_screen_idx = APIs.Memory.ReadU16(0x1d8c00 + 0x01e8, "RDRAM");
        if (pause_screen_idx != 0) { return; }
        var pause_changing = APIs.Memory.ReadU16(0x1d8c00 + 0x01e4, "RDRAM");
        if (pause_changing != 0 && pause_changing != 3) { return; }
        // not CAN_DRAW_TRADE_DPAD
        var pause_item_cursor = APIs.Memory.ReadS16(0x1d8c00 + 0x0218, "RDRAM");
        if (pause_item_cursor == 0x16) {
            // Z64_SLOT_ADULT_TRADE
            // assume CFG_ADULT_TRADE_SHUFFLE
            //TODO check via https://github.com/OoTRandomizer/OoT-Randomizer/pull/2156
            return;
        } else if (pause_item_cursor == 0x17) {
            // Z64_SLOT_CHILD_TRADE
            // assume CFG_CHILD_TRADE_SHUFFLE
            //TODO check via https://github.com/OoTRandomizer/OoT-Randomizer/pull/2156
            return;
        }
        // draw
        uint cosmeticsCtxVersion = 0;
        if (cosmeticsCtxAddr != 0) {
            cosmeticsCtxVersion = APIs.Memory.ReadU32(cosmeticsCtxAddr, "System Bus");
        }
        var cfg_dpad_dungeon_info_enable = false;
        if (cosmeticsCtxVersion >= 0x1f073fd9) {
            cfg_dpad_dungeon_info_enable = APIs.Memory.ReadByte(cosmeticsCtxAddr + 0x0055, "System Bus") != 0;
        }
        var pad_held = APIs.Memory.ReadU16(0x1c84b4, "RDRAM");
        bool d_down_held = (pad_held & 0x0400) != 0;
        bool a_held = (pad_held & 0x8000) != 0;
        if (!(cfg_dpad_dungeon_info_enable && d_down_held) && !a_held) { return; }
        // menus
        var cfg_dungeon_info_reward_enable = APIs.Memory.ReadU32(trackerCtxAddr + 0x10, "System Bus") != 0;
        var cfg_dungeon_info_reward_need_compass = APIs.Memory.ReadU32(trackerCtxAddr + 0x14, "System Bus");
        var cfg_dungeon_info_reward_need_altar = APIs.Memory.ReadU32(trackerCtxAddr + 0x18, "System Bus") != 0;
        var show_stones = cfg_dungeon_info_reward_enable && (!cfg_dungeon_info_reward_need_altar || (APIs.Memory.ReadByte(0x11a5d0 + 0x0ef8 + 55, "RDRAM") & 2) != 0);
        var show_meds = cfg_dungeon_info_reward_enable && (!cfg_dungeon_info_reward_need_altar || (APIs.Memory.ReadByte(0x11a5d0 + 0x0ef8 + 55, "RDRAM") & 1) != 0);
        if (a_held && !(d_down_held && cfg_dpad_dungeon_info_enable)) {
            // A menu
            var cfg_dungeon_info_reward_summary_enable = APIs.Memory.ReadU32(trackerCtxAddr + 0x1c, "System Bus") != 0;
            if (!cfg_dungeon_info_reward_summary_enable) { return; }
            byte emerald_world = 0;
            OptHintArea emerald_area = OptHintArea.Unknown;
            byte ruby_world = 0;
            OptHintArea ruby_area = OptHintArea.Unknown;
            byte sapphire_world = 0;
            OptHintArea sapphire_area = OptHintArea.Unknown;
            byte light_world = 0;
            OptHintArea light_area = OptHintArea.Unknown;
            byte forest_world = 0;
            OptHintArea forest_area = OptHintArea.Unknown;
            byte fire_world = 0;
            OptHintArea fire_area = OptHintArea.Unknown;
            byte water_world = 0;
            OptHintArea water_area = OptHintArea.Unknown;
            byte shadow_world = 0;
            OptHintArea shadow_area = OptHintArea.Unknown;
            byte spirit_world = 0;
            OptHintArea spirit_area = OptHintArea.Unknown;
            for (byte dungeon_idx = 0; dungeon_idx < 14; dungeon_idx++) {
                if (cfg_dungeon_info_reward_need_compass == 0 || (APIs.Memory.ReadByte(0x11a5d0 + 0x00a8 + dungeon_idx, "RDRAM") & 2) != 0) {
                    var reward = APIs.Memory.ReadByte(trackerCtxAddr + 0x20 + dungeon_idx, "System Bus");
                    if (reward < 0) {
                        // none or unknown
                    } else if (reward < 3) {
                        if (show_stones) {
                            switch (reward) {
                                case 0: {
                                    emerald_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    emerald_world = playerID;
                                    break;
                                }
                                case 1: {
                                    ruby_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    ruby_world = playerID;
                                    break;
                                }
                                case 2: {
                                    sapphire_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    sapphire_world = playerID;
                                    break;
                                }
                            }
                        }
                    } else {
                        if (show_meds) {
                            switch (reward) {
                                case 3: {
                                    forest_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    forest_world = playerID;
                                    break;
                                }
                                case 4: {
                                    fire_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    fire_world = playerID;
                                    break;
                                }
                                case 5: {
                                    water_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    water_world = playerID;
                                    break;
                                }
                                case 6: {
                                    spirit_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    spirit_world = playerID;
                                    break;
                                }
                                case 7: {
                                    shadow_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    shadow_world = playerID;
                                    break;
                                }
                                case 8: {
                                    light_area = HintAreaFromDungeonIdx(dungeon_idx);
                                    light_world = playerID;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Native.client_send_dungeon_reward_info(
                emerald_world, emerald_area,
                ruby_world, ruby_area,
                sapphire_world, sapphire_area,
                light_world, light_area,
                forest_world, forest_area,
                fire_world, fire_area,
                water_world, water_area,
                shadow_world, shadow_area,
                spirit_world, spirit_area,
                client
            );
        } else if (d_down_held) {
            // D-down menu
            byte emerald_world = 0;
            OptHintArea emerald_area = OptHintArea.Unknown;
            byte ruby_world = 0;
            OptHintArea ruby_area = OptHintArea.Unknown;
            byte sapphire_world = 0;
            OptHintArea sapphire_area = OptHintArea.Unknown;
            byte light_world = 0;
            OptHintArea light_area = OptHintArea.Unknown;
            byte forest_world = 0;
            OptHintArea forest_area = OptHintArea.Unknown;
            byte fire_world = 0;
            OptHintArea fire_area = OptHintArea.Unknown;
            byte water_world = 0;
            OptHintArea water_area = OptHintArea.Unknown;
            byte shadow_world = 0;
            OptHintArea shadow_area = OptHintArea.Unknown;
            byte spirit_world = 0;
            OptHintArea spirit_area = OptHintArea.Unknown;
            for (byte i = 0; i < 9; i++) {
                if (i < 3 ? show_stones : show_meds) {
                    byte reward = REWARD_ROWS[i];
                    bool display_area = true;
                    switch (cfg_dungeon_info_reward_need_compass) {
                        case 1: {
                            for (int dungeon_idx = 0; dungeon_idx < 8; dungeon_idx++) {
                                if (APIs.Memory.ReadByte(trackerCtxAddr + 0x20 + dungeon_idx, "System Bus") == reward) {
                                    if ((APIs.Memory.ReadByte(0x11a5d0 + 0x00a8 + dungeon_idx, "RDRAM") & 2) == 0) {
                                        display_area = false;
                                    }
                                    break;
                                }
                            }
                            break;
                        }
                        case 2: {
                            if (i != 3) {
                                byte dungeon_idx = REWARD_ROWS[i];
                                display_area = (APIs.Memory.ReadByte(0x11a5d0 + 0x00a8 + dungeon_idx, "RDRAM") & 2) != 0;
                            }
                            break;
                        }
                    }
                    if (display_area) {
                        var area = HintAreaFromRewardInfo(trackerCtxAddr, i);
                        var world = playerID; //TODO add CFG_DUNGEON_INFO_REWARD_WORLDS_ENABLE and CFG_DUNGEON_REWARD_WORLDS to tracker context as part of dungeon reward shuffle PR
                        switch (reward) {
                            case 0: {
                                emerald_area = area;
                                emerald_world = world;
                                break;
                            }
                            case 1: {
                                ruby_area = area;
                                ruby_world = world;
                                break;
                            }
                            case 2: {
                                sapphire_area = area;
                                sapphire_world = world;
                                break;
                            }
                            case 3: {
                                forest_area = area;
                                forest_world = world;
                                break;
                            }
                            case 4: {
                                fire_area = area;
                                fire_world = world;
                                break;
                            }
                            case 5: {
                                water_area = area;
                                water_world = world;
                                break;
                            }
                            case 6: {
                                spirit_area = area;
                                spirit_world = world;
                                break;
                            }
                            case 7: {
                                shadow_area = area;
                                shadow_world = world;
                                break;
                            }
                            case 8: {
                                light_area = area;
                                light_world = world;
                                break;
                            }
                        }
                    }
                }
            }
            Native.client_send_dungeon_reward_info(
                emerald_world, emerald_area,
                ruby_world, ruby_area,
                sapphire_world, sapphire_area,
                light_world, light_area,
                forest_world, forest_area,
                fire_world, fire_area,
                water_world, water_area,
                shadow_world, shadow_area,
                spirit_world, spirit_area,
                client
            );
        }
    }

    private void SetError(Error error) {
        using (var debug = error.Debug()) {
            using (var display = error.Display()) {
                this.DialogController.ShowMessageBox(this, $"{display.AsString()}\n\ndebug info: {debug.AsString()}", "Error in Mido's House Multiworld for BizHawk", EMsgBoxIcon.Error);
            }
        }
        if (this.client != null) {
            this.client.Dispose();
            this.client = null;
        }
        this.Close();
    }
}
