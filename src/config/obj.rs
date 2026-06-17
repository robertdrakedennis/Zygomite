use super::{parse_param_ops, OpListEntry};
use crate::cache_bail as bail;
use crate::error::Result;
use crate::packet::Packet;

pub fn parse_obj(id: u32, data: &[u8]) -> Result<OpListEntry> {
    let mut packet = Packet::new(data);
    let mut ops = Vec::new();

    loop {
        match packet.g1()? {
            0 => {
                if !packet.is_done() {
                    bail!("obj {id} did not consume full payload");
                }
                return Ok(OpListEntry { id, ops });
            }
            1 => ops.push(format!("model={}", packet.gsmart2or4null()?)),
            2 => ops.push(format!("name={}", packet.gjstr()?)),
            3 => ops.push(format!("desc={}", packet.gjstr()?)),
            4 => ops.push(format!("2dzoom={}", packet.g2()?)),
            5 => ops.push(format!("2dxan={}", packet.g2()?)),
            6 => ops.push(format!("2dyan={}", packet.g2()?)),
            7 => ops.push(format!("2dxof={}", packet.g2s()?)),
            8 => ops.push(format!("2dyof={}", packet.g2s()?)),
            9 => ops.push(format!("unknown9={}", packet.gjstr()?)),
            10 => ops.push(format!("anim={}", packet.g2()?)),
            11 => ops.push(String::from("stackable=yes")),
            12 => ops.push(format!("cost={}", packet.g4s()?)),
            13 => ops.push(format!("wearpos={}", packet.g1()?)),
            14 => ops.push(format!("wearpos2={}", packet.g1()?)),
            15 => ops.push(String::from("tradeable=no")),
            16 => ops.push(String::from("members=yes")),
            23 => ops.push(format!("manwear={}", packet.gsmart2or4null()?)),
            24 => ops.push(format!("manwear2={}", packet.gsmart2or4null()?)),
            25 => ops.push(format!("womanwear={}", packet.gsmart2or4null()?)),
            26 => ops.push(format!("womanwear2={}", packet.gsmart2or4null()?)),
            27 => ops.push(format!("wearpos3={}", packet.g1()?)),
            30 => ops.push(format!("op1={}", packet.gjstr()?)),
            31 => ops.push(format!("op2={}", packet.gjstr()?)),
            32 => ops.push(format!("op3={}", packet.gjstr()?)),
            33 => ops.push(format!("op4={}", packet.gjstr()?)),
            34 => ops.push(format!("op5={}", packet.gjstr()?)),
            35 => ops.push(format!("iop1={}", packet.gjstr()?)),
            36 => ops.push(format!("iop2={}", packet.gjstr()?)),
            37 => ops.push(format!("iop3={}", packet.gjstr()?)),
            38 => ops.push(format!("iop4={}", packet.gjstr()?)),
            39 => ops.push(format!("iop5={}", packet.gjstr()?)),
            40 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("recol{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("recol{}d={}", i + 1, packet.g2()?));
                }
            }
            41 => {
                let count = usize::from(packet.g1()?);
                for i in 0..count {
                    ops.push(format!("retex{}s={}", i + 1, packet.g2()?));
                    ops.push(format!("retex{}d={}", i + 1, packet.g2()?));
                }
            }
            42 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("unknown42={}", packet.g1s()?));
                }
            }
            43 => ops.push(format!("minimenucolour={}", packet.g4s()?)),
            44 => ops.push(format!("recolindices={}", packet.g2()?)),
            45 => ops.push(format!("retexindices={}", packet.g2()?)),
            65 => ops.push(String::from("stockmarket=yes")),
            69 => ops.push(format!("stockmarketlimit={}", packet.g4s()?)),
            78 => ops.push(format!("manwear3={}", packet.gsmart2or4null()?)),
            79 => ops.push(format!("womanwear3={}", packet.gsmart2or4null()?)),
            90 => ops.push(format!("manhead={}", packet.gsmart2or4null()?)),
            91 => ops.push(format!("womanhead={}", packet.gsmart2or4null()?)),
            92 => ops.push(format!("manhead2={}", packet.gsmart2or4null()?)),
            93 => ops.push(format!("womanhead2={}", packet.gsmart2or4null()?)),
            94 => ops.push(format!("category={}", packet.g2()?)),
            95 => ops.push(format!("2dzan={}", packet.g2()?)),
            96 => ops.push(format!("dummyitem={}", packet.g1()?)),
            97 => ops.push(format!("certlink={}", packet.g2()?)),
            98 => ops.push(format!("certtemplate={}", packet.g2()?)),
            100 => ops.push(format!("count1={},{}", packet.g2()?, packet.g2()?)),
            101 => ops.push(format!("count2={},{}", packet.g2()?, packet.g2()?)),
            102 => ops.push(format!("count3={},{}", packet.g2()?, packet.g2()?)),
            103 => ops.push(format!("count4={},{}", packet.g2()?, packet.g2()?)),
            104 => ops.push(format!("count5={},{}", packet.g2()?, packet.g2()?)),
            105 => ops.push(format!("count6={},{}", packet.g2()?, packet.g2()?)),
            106 => ops.push(format!("count7={},{}", packet.g2()?, packet.g2()?)),
            107 => ops.push(format!("count8={},{}", packet.g2()?, packet.g2()?)),
            108 => ops.push(format!("count9={},{}", packet.g2()?, packet.g2()?)),
            109 => ops.push(format!("count10={},{}", packet.g2()?, packet.g2()?)),
            110 => ops.push(format!("resizex={}", packet.g2()?)),
            111 => ops.push(format!("resizey={}", packet.g2()?)),
            112 => ops.push(format!("resizez={}", packet.g2()?)),
            113 => ops.push(format!("ambient={}", packet.g1s()?)),
            114 => ops.push(format!("contrast={}", packet.g1s()?)),
            115 => ops.push(format!("team={}", packet.g1()?)),
            121 => ops.push(format!("lentlink={}", packet.g2()?)),
            122 => ops.push(format!("lenttemplate={}", packet.g2()?)),
            125 => ops.push(format!(
                "manwearoff={},{},{}",
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?
            )),
            126 => ops.push(format!(
                "womanwearoff={},{},{}",
                packet.g1s()?,
                packet.g1s()?,
                packet.g1s()?
            )),
            127 => ops.push(format!("cursor1={},{}", packet.g1()?, packet.g2()?)),
            128 => ops.push(format!("cursor2={},{}", packet.g1()?, packet.g2()?)),
            129 => ops.push(format!("icursor1={},{}", packet.g1()?, packet.g2()?)),
            130 => ops.push(format!("icursor2={},{}", packet.g1()?, packet.g2()?)),
            131 => ops.push(format!("unknown131={}", packet.gjstr()?)),
            132 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    ops.push(format!("quest={}", packet.g2()?));
                }
            }
            134 => ops.push(format!("picksizeshift={}", packet.g1()?)),
            139 => ops.push(format!("boughtlink={}", packet.g2()?)),
            140 => ops.push(format!("boughttemplate={}", packet.g2()?)),
            142 => ops.push(format!("cursor1={}", packet.g2()?)),
            143 => ops.push(format!("cursor2={}", packet.g2()?)),
            144 => ops.push(format!("cursor3={}", packet.g2()?)),
            145 => ops.push(format!("cursor4={}", packet.g2()?)),
            146 => ops.push(format!("cursor5={}", packet.g2()?)),
            148 => ops.push(format!("placeholderlink={}", packet.g2()?)),
            149 => ops.push(format!("placeholdertemplate={}", packet.g2()?)),
            150 => ops.push(format!("icursor1={}", packet.g2()?)),
            151 => ops.push(format!("icursor2={}", packet.g2()?)),
            152 => ops.push(format!("icursor3={}", packet.g2()?)),
            153 => ops.push(format!("icursor4={}", packet.g2()?)),
            154 => ops.push(format!("icursor5={}", packet.g2()?)),
            156 => ops.push(String::from("shadow=no")),
            157 => ops.push(String::from("unknown157=yes")),
            161 => ops.push(format!("shardlink={}", packet.g2()?)),
            162 => ops.push(format!("shardtemplate={}", packet.g2()?)),
            163 => ops.push(format!("shardcount={}", packet.g2()?)),
            164 => ops.push(format!("shardname={}", packet.gjstr()?)),
            165 => ops.push(String::from("stackable=never")),
            167 => ops.push(String::from("unknown167=yes")),
            168 => ops.push(String::from("placeholder=no")),
            178 => ops.push(String::from("stackable=sometimes")),
            181 => ops.push(format!("cost={}", packet.g8s()?)),
            249 => parse_param_ops(&mut packet, &mut ops)?,
            opcode => bail!("unknown obj opcode {opcode} in {id}"),
        }
    }
}
