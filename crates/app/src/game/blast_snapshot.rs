//! Persist immutable blast-planning geometry. Losing this snapshot and starting
//! again from the post-save world changes shielding after edits or another blast.
use super::BlastSnapshot;
use mc4d_core::{block::BlockState,chunk::{Chunk,CHUNK_VOLUME},glam::IVec4};
use serde::{Serialize,Deserialize,Serializer,Deserializer};
#[derive(Serialize,Deserialize)]struct FrozenChunk {position:[i32;4],runs:String}
fn local(i:usize)->IVec4{IVec4::new((i&15)as i32,((i>>4)&15)as i32,((i>>8)&15)as i32,((i>>12)&15)as i32)}
fn encode(chunk:&Chunk)->String{
 const HEX:&[u8;16]=b"0123456789abcdef";
 let mut text=String::new();let mut previous=chunk.get_state(IVec4::ZERO);let mut count=0u32;
 let mut emit=|s:BlockState,n:u32|{for byte in [s.id,s.metadata].into_iter().chain(n.to_le_bytes()){text.push(HEX[(byte>>4)as usize]as char);text.push(HEX[(byte&15)as usize]as char);}};
 if chunk.uniform_id().is_some()&&chunk.metadata_plane().is_none(){emit(previous,CHUNK_VOLUME as u32);return text;}
 for i in 0..CHUNK_VOLUME {let state=chunk.get_state(local(i));if state!=previous {emit(previous,count);previous=state;count=0;}count+=1;}
 emit(previous,count);text
}
fn decode(text:&str)->Result<Chunk,String>{
 if text.is_empty()||text.len()%12!=0||text.len()>CHUNK_VOLUME*12{return Err("invalid frozen blast run data".into());}
 let nibble=|b:u8|match b{b'0'..=b'9'=>Some(b-b'0'),b'a'..=b'f'=>Some(b-b'a'+10),b'A'..=b'F'=>Some(b-b'A'+10),_=>None};
 let mut blocks=Vec::with_capacity(CHUNK_VOLUME);let mut metadata=Vec::with_capacity(CHUNK_VOLUME);
 for row in text.as_bytes().chunks_exact(12){let mut bytes=[0u8;6];for(i,pair)in row.chunks_exact(2).enumerate(){bytes[i]=nibble(pair[0]).ok_or("invalid frozen blast hex")?*16+nibble(pair[1]).ok_or("invalid frozen blast hex")?;}
  let count=u32::from_le_bytes(bytes[2..6].try_into().unwrap())as usize;if count==0||count>CHUNK_VOLUME-blocks.len(){return Err("invalid frozen blast run length".into());}
  blocks.extend(std::iter::repeat_n(bytes[0],count));metadata.extend(std::iter::repeat_n(bytes[1],count));
 }
 if blocks.len()!=CHUNK_VOLUME{return Err("incomplete frozen blast chunk".into());}
 Chunk::from_planes(blocks,Some(metadata)).ok_or_else(||"invalid frozen blast planes".to_string())
}
pub fn serialize<S:Serializer>(value:&Option<BlastSnapshot>,serializer:S)->Result<S::Ok,S::Error>{
 let frozen=value.as_ref().map(|snapshot|{let mut chunks:Vec<_>=snapshot.iter().map(|(p,c)|FrozenChunk{position:p.to_array(),runs:encode(c)}).collect();chunks.sort_by_key(|c|c.position);chunks});frozen.serialize(serializer)
}
pub fn deserialize<'de,D:Deserializer<'de>>(deserializer:D)->Result<Option<BlastSnapshot>,D::Error>{
 let chunks=Option::<Vec<FrozenChunk>>::deserialize(deserializer)?;
 chunks.map(|chunks|{if chunks.len()>256{return Err(serde::de::Error::custom("too many frozen blast chunks"));}let mut result=BlastSnapshot::new();for chunk in chunks{let position=IVec4::from_array(chunk.position);let decoded=decode(&chunk.runs).map_err(serde::de::Error::custom)?;if result.insert(position,decoded).is_some(){return Err(serde::de::Error::custom("duplicate frozen blast chunk"));}}Ok(result)}).transpose()
}
#[cfg(test)]mod tests{
 use super::*;use mc4d_core::block;
 #[test]fn metadata_and_uniform_geometry_roundtrip_without_loss(){let mut c=Chunk::uniform(block::AIR);c.set_state(IVec4::new(4,5,6,7),BlockState::new(block::WOOD_DOOR,20));let restored=decode(&encode(&c)).unwrap();for i in 0..CHUNK_VOLUME{assert_eq!(c.get_state(local(i)),restored.get_state(local(i)));}assert_eq!(encode(&Chunk::uniform(block::STONE)).len(),12);}
 #[test]fn malformed_frozen_geometry_is_rejected(){for text in ["","000000000000","000001000000","zz0000000100","000000000200"]{assert!(decode(text).is_err(),"{text}");}}
}
