use {
    std::{
        fs,
        num::NonZeroU8,
        time::Duration,
    },
    itertools::Itertools as _,
    syn::{
        Expr,
        ExprLit,
        ExprUnary,
        FieldValue,
        Lit,
        Member,
        UnOp,
    },
    multiworld::{
        ClientMessage,
        Filename,
        HashIcon,
        SpoilerLog,
    },
    crate::Error,
};

pub(crate) trait FromExpr: Sized {
    fn from_expr(expr: Expr) -> Result<Self, Error>;
}

impl FromExpr for u64 {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok(lit.base10_parse()?),
            Expr::Unary(ExprUnary { op: UnOp::Neg(_), expr, .. }) => match *expr {
                Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok((-lit.base10_parse::<i64>()?) as u64),
                _ => Err(Error::FromExpr),
            },
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for u8 {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok(lit.base10_parse()?),
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for NonZeroU8 {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok(lit.base10_parse()?),
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for u16 {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok(lit.base10_parse()?),
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for u32 {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) => Ok(lit.base10_parse()?),
            _ => Err(Error::FromExpr),
        }
    }
}

impl<T: FromExpr> FromExpr for Option<T> {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Call(call) => match *call.func {
                Expr::Path(path) => if let Some(ident) = path.path.get_ident() {
                    match &*ident.to_string() {
                        "Some" => {
                            let content = call.args.into_iter().exactly_one()?;
                            Ok(Some(T::from_expr(content)?))
                        }
                        _ => Err(Error::FromExpr),
                    }
                } else {
                    Err(Error::FromExpr)
                },
                _ => Err(Error::FromExpr),
            },
            Expr::Path(path) => if let Some(ident) = path.path.get_ident() {
                match &*ident.to_string() {
                    "None" => Ok(None),
                    _ => Err(Error::FromExpr),
                }
            } else {
                Err(Error::FromExpr)
            },
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for String {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) => Ok(lit.value()),
            _ => Err(Error::FromExpr),
        }
    }
}

impl<T: FromExpr, const N: usize> FromExpr for [T; N] {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Array(array) => {
                let mut buf = Vec::with_capacity(N);
                for elt in array.elems {
                    buf.push(T::from_expr(elt)?);
                }
                Ok(Self::try_from(buf).map_err(|_| Error::FromExpr)?)
            }
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for Duration {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Call(call) => match *call.func {
                Expr::Path(path) => if path.path.segments.into_iter().map(|seg| seg.ident.to_string()).eq(["Duration", "new"]) {
                    let (secs, nanos) = call.args.into_iter().collect_tuple().ok_or(Error::FromExpr)?;
                    Ok(Self::new(u64::from_expr(secs)?, u32::from_expr(nanos)?))
                } else {
                    Err(Error::FromExpr) //TODO support other Duration constructors
                },
                _ => Err(Error::FromExpr),
            },
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for Filename {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            //Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) => Ok(lit.value().parse()?), //TODO allow filename input as string literal
            Expr::Array(array) => Ok(Self(<_>::from_expr(Expr::Array(array))?)),
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for HashIcon {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Path(path) => if let Some(ident) = path.path.get_ident() {
                match &*ident.to_string() {
                    "DekuStick" => Ok(Self::DekuStick),
                    "DekuNut" => Ok(Self::DekuNut),
                    "Bow" => Ok(Self::Bow),
                    "Slingshot" => Ok(Self::Slingshot),
                    "FairyOcarina" => Ok(Self::FairyOcarina),
                    "Bombchu" => Ok(Self::Bombchu),
                    "Longshot" => Ok(Self::Longshot),
                    "Boomerang" => Ok(Self::Boomerang),
                    "LensOfTruth" => Ok(Self::LensOfTruth),
                    "Beans" => Ok(Self::Beans),
                    "MegatonHammer" => Ok(Self::MegatonHammer),
                    "BottledFish" => Ok(Self::BottledFish),
                    "BottledMilk" => Ok(Self::BottledMilk),
                    "MaskOfTruth" => Ok(Self::MaskOfTruth),
                    "SoldOut" => Ok(Self::SoldOut),
                    "Cucco" => Ok(Self::Cucco),
                    "Mushroom" => Ok(Self::Mushroom),
                    "Saw" => Ok(Self::Saw),
                    "Frog" => Ok(Self::Frog),
                    "MasterSword" => Ok(Self::MasterSword),
                    "MirrorShield" => Ok(Self::MirrorShield),
                    "KokiriTunic" => Ok(Self::KokiriTunic),
                    "HoverBoots" => Ok(Self::HoverBoots),
                    "SilverGauntlets" => Ok(Self::SilverGauntlets),
                    "GoldScale" => Ok(Self::GoldScale),
                    "StoneOfAgony" => Ok(Self::StoneOfAgony),
                    "SkullToken" => Ok(Self::SkullToken),
                    "HeartContainer" => Ok(Self::HeartContainer),
                    "BossKey" => Ok(Self::BossKey),
                    "Compass" => Ok(Self::Compass),
                    "Map" => Ok(Self::Map),
                    "BigMagic" => Ok(Self::BigMagic),
                    _ => Err(Error::FromExpr),
                }
            } else {
                Err(Error::FromExpr)
            },
            _ => Err(Error::FromExpr),
        }
    }
}

impl FromExpr for SpoilerLog {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        let path = String::from_expr(expr)?;
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }
}

impl FromExpr for ClientMessage {
    fn from_expr(expr: Expr) -> Result<Self, Error> {
        match expr {
            Expr::Call(call) => match *call.func {
                Expr::Path(path) => if let Some(ident) = path.path.get_ident() {
                    match &*ident.to_string() {
                        "PlayerId" => {
                            let world_id = call.args.into_iter().exactly_one()?;
                            Ok(Self::PlayerId(NonZeroU8::from_expr(world_id)?))
                        }
                        "PlayerName" => {
                            let filename = call.args.into_iter().exactly_one()?;
                            Ok(Self::PlayerName(Filename::from_expr(filename)?))
                        }
                        "KickPlayer" => {
                            let world_id = call.args.into_iter().exactly_one()?;
                            Ok(Self::KickPlayer(NonZeroU8::from_expr(world_id)?))
                        }
                        //TODO SaveData (read from path?)
                        "FileHash" => {
                            let hash = call.args.into_iter().exactly_one()?;
                            Ok(Self::FileHash(<[HashIcon; 5]>::from_expr(hash)?))
                        }
                        "AutoDeleteDelta" => {
                            let interval = call.args.into_iter().exactly_one()?;
                            Ok(Self::AutoDeleteDelta(Duration::from_expr(interval)?))
                        }
                        _ => Err(Error::FromExpr),
                    }
                } else {
                    Err(Error::FromExpr)
                },
                _ => Err(Error::FromExpr),
            },
            Expr::Path(path) => if let Some(ident) = path.path.get_ident() {
                match &*ident.to_string() {
                    "Ping" => Ok(Self::Ping),
                    "Stop" => Ok(Self::Stop),
                    "ResetPlayerId" => Ok(Self::ResetPlayerId),
                    "DeleteRoom" => Ok(Self::DeleteRoom),
                    _ => Err(Error::FromExpr),
                }
            } else {
                Err(Error::FromExpr)
            },
            Expr::Struct(struct_lit) => if let Some(ident) = struct_lit.path.get_ident() {
                match &*ident.to_string() {
                    "JoinRoom" => {
                        let mut name = None;
                        let mut password = None;
                        for FieldValue { member, expr, .. } in struct_lit.fields {
                            match member {
                                Member::Named(member) => match &*member.to_string() {
                                    "name" => if name.replace(String::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "password" => if password.replace(Option::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    _ => return Err(Error::FromExpr),
                                },
                                Member::Unnamed(_) => return Err(Error::FromExpr),
                            }
                        }
                        Ok(Self::JoinRoom { name: name.ok_or(Error::FromExpr)?, password: password.ok_or(Error::FromExpr)? })
                    }
                    "CreateRoom" => {
                        let mut name = None;
                        let mut password = None;
                        for FieldValue { member, expr, .. } in struct_lit.fields {
                            match member {
                                Member::Named(member) => match &*member.to_string() {
                                    "name" => if name.replace(String::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "password" => if password.replace(String::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    _ => return Err(Error::FromExpr),
                                },
                                Member::Unnamed(_) => return Err(Error::FromExpr),
                            }
                        }
                        Ok(Self::CreateRoom { name: name.ok_or(Error::FromExpr)?, password: password.ok_or(Error::FromExpr)? })
                    }
                    "Login" => {
                        let mut id = None;
                        let mut api_key = None;
                        for FieldValue { member, expr, .. } in struct_lit.fields {
                            match member {
                                Member::Named(member) => match &*member.to_string() {
                                    "id" => if id.replace(u64::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "api_key" => if api_key.replace(<_>::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    _ => return Err(Error::FromExpr),
                                },
                                Member::Unnamed(_) => return Err(Error::FromExpr),
                            }
                        }
                        Ok(Self::Login { id: id.ok_or(Error::FromExpr)?, api_key: api_key.ok_or(Error::FromExpr)? })
                    }
                    "SendItem" => {
                        let mut key = None;
                        let mut kind = None;
                        let mut target_world = None;
                        for FieldValue { member, expr, .. } in struct_lit.fields {
                            match member {
                                Member::Named(member) => match &*member.to_string() {
                                    "key" => if key.replace(u32::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "kind" => if kind.replace(u16::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "target_world" => if target_world.replace(NonZeroU8::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    _ => return Err(Error::FromExpr),
                                },
                                Member::Unnamed(_) => return Err(Error::FromExpr),
                            }
                        }
                        Ok(Self::SendItem { key: key.ok_or(Error::FromExpr)?, kind: kind.ok_or(Error::FromExpr)?, target_world: target_world.ok_or(Error::FromExpr)? })
                    }
                    "Track" => {
                        let mut mw_room_name = None;
                        let mut tracker_room_name = None;
                        let mut world_count = None;
                        for FieldValue { member, expr, .. } in struct_lit.fields {
                            match member {
                                Member::Named(member) => match &*member.to_string() {
                                    "mw_room_name" => if mw_room_name.replace(String::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "tracker_room_name" => if tracker_room_name.replace(String::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "world_count" => if world_count.replace(NonZeroU8::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    _ => return Err(Error::FromExpr),
                                },
                                Member::Unnamed(_) => return Err(Error::FromExpr),
                            }
                        }
                        Ok(Self::Track { mw_room_name: mw_room_name.ok_or(Error::FromExpr)?, tracker_room_name: tracker_room_name.ok_or(Error::FromExpr)?, world_count: world_count.ok_or(Error::FromExpr)? })
                    }
                    "SendAll" => {
                        let mut room = None;
                        let mut source_world = None;
                        let mut spoiler_log = None;
                        for FieldValue { member, expr, .. } in struct_lit.fields {
                            match member {
                                Member::Named(member) => match &*member.to_string() {
                                    "room" => if room.replace(String::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "source_world" => if source_world.replace(NonZeroU8::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    "spoiler_log" => if spoiler_log.replace(SpoilerLog::from_expr(expr)?).is_some() { return Err(Error::FromExpr) },
                                    _ => return Err(Error::FromExpr),
                                }
                                Member::Unnamed(_) => return Err(Error::FromExpr),
                            }
                        }
                        Ok(Self::SendAll { room: room.ok_or(Error::FromExpr)?, source_world: source_world.ok_or(Error::FromExpr)?, spoiler_log: spoiler_log.ok_or(Error::FromExpr)? })
                    }
                    //TODO TrackError
                    _ => Err(Error::FromExpr),
                }
            } else {
                Err(Error::FromExpr)
            },
            _ => Err(Error::FromExpr),
        }
    }
}
