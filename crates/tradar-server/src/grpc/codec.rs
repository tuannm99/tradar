use super::*;

/// `tonic::codec::ProstCodec` (the one `tonic`'s own generated clients use)
/// needs `T: Default` to decode -- it builds `T::default()` then merges
/// into it. `DynamicMessage` has no such `Default`: it needs a
/// `MessageDescriptor` at construction time, which differs per method. This
/// codec carries that descriptor instead.
#[derive(Clone)]
pub(crate) struct DynamicCodec {
    pub(crate) response_desc: prost_reflect::MessageDescriptor,
}

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            desc: self.response_desc.clone(),
        }
    }
}

pub(crate) struct DynamicEncoder;

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|e| Status::internal(format!("encode error: {e}")))
    }
}

pub(crate) struct DynamicDecoder {
    desc: prost_reflect::MessageDescriptor,
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        if !src.has_remaining() {
            return Ok(None);
        }
        let msg = DynamicMessage::decode(self.desc.clone(), src)
            .map_err(|e| Status::internal(format!("decode error: {e}")))?;
        Ok(Some(msg))
    }
}
