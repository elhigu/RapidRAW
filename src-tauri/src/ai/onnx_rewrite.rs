//! Makes AI models run faster on x86-64 CPUs by replacing their stride-1 `ConvTranspose` layers
//! with `Conv` layers that compute the same result. Every model goes through `optimized_model` when
//! it is loaded, which uses the rewrite only where it was measured to help (see the table below).
//! Of the current models, only the denoise model has such layers. The model is changed in memory;
//! the file on disk is not changed.
//!
//! Why: the denoise model spends most of its time in `ConvTranspose` layers. On x86-64 CPUs, ONNX
//! Runtime runs `Conv` layers with its fastest kernels, which use its "NCHWc" memory layout: the
//! image is stored in blocks of 8 or 16 channels, so one SIMD instruction works on the same pixel
//! in 8 or 16 channels at once. ONNX Runtime has no such kernels for `ConvTranspose`, and it
//! doesn't make this replacement by itself. On ARM CPUs the rewrite makes no difference, and on
//! Apple Silicon it is slower, so it's only used on x86-64 CPUs. It's not used with DirectML or
//! CoreML either. Measured time for one 504×504 denoise tile, with the original and the rewritten
//! model:
//!
//! ```text
//! Hardware, OS                   Runs on   ORT   Original  Rewritten  Change  Used
//! Ryzen AI 9 HX 375, Linux       CPU       1.32  0.93 s    0.75 s     -19%    yes
//! Xeon 6975P, Linux (CI)         CPU       1.22  1.26 s    0.95 s     -24%    yes
//! Core i7-8700B, macOS (CI)      CPU       1.22  2.86 s    2.15 s     -25%    yes
//! EPYC 7763, Windows (CI)        CPU       1.22  2.42 s    2.30 s     -5%     yes
//! Neoverse-N2, Linux (CI)        CPU       1.22  1.99 s    1.97 s     -1%     no
//! Cobalt 100, Windows (CI)       CPU       1.32  2.02 s    2.00 s     -1%     no
//! Apple M2 Pro, macOS (CI)       CPU       1.22  2.04 s    2.42 s     +18%    no
//! Apple M2 Pro, macOS (CI)       CoreML    1.32  0.45 s    0.44 s     -1%     no
//! Core i7-8700B, macOS (CI)      CoreML    1.22  2.08 s    1.86 s     -10%    no
//! NVIDIA/AMD/Intel GPU, Windows  DirectML  not tested                         no
//! ```
//!
//! ORT is the ONNX Runtime version. Rows marked (CI) were measured on GitHub Actions runners,
//! which are virtual machines with 3 to 5 threads, because we don't have that hardware. CoreML
//! used the same settings as the app (MLProgram, all compute units). Please add results for other
//! platforms, so that the rewrite can be used wherever it helps. To measure, save the rewritten
//! model to a file and time one tile with both models in ONNX Runtime.
//!
//! How: a `Conv` computes each output pixel from its neighbouring input pixels. A `ConvTranspose`
//! does the opposite: it spreads each input pixel to its neighbouring output pixels. With stride 1
//! both add up the same products, only with the kernel in the opposite order. For a 3-wide kernel
//! `[a b c]` and padding 1:
//!
//! ```text
//! ConvTranspose: x1 adds a*x1, b*x1, c*x1 to y0, y1, y2, and so on, so  y2 = c*x1 + b*x2 + a*x3
//! Conv with the kernel flipped to [c b a]:                             y2 = c*x1 + b*x2 + a*x3
//! ```
//!
//! So each `ConvTranspose` is replaced by a `Conv` with:
//! - the kernel flipped horizontally and vertically,
//! - the first two axes of the weights swapped, because `ConvTranspose` stores its weights as
//!   (input channel, output channel, height, width) and `Conv` as (output channel, input channel,
//!   height, width),
//! - padding `k - 1 - p` instead of `p`, where `k` is the kernel size, so the output keeps its
//!   size,
//! - the same bias.
//!
//! Why the result stays the same: every output pixel still gets exactly the same products added
//! together. Only the order of the additions can change, and that changes only floating-point
//! rounding. On a denoise tile the largest difference was 3.6e-7.
//!
//! A layer is only replaced if it has stride 1, no groups, dilations, output padding or automatic
//! padding, padding between 0 and `k - 1`, and float weights that no other layer uses, because
//! the weights are changed in place. Everything else in the model is left as it is. The ONNX file
//! format (protobuf) is read and written directly in this file.

use anyhow::{Result, anyhow, bail, ensure};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const MODEL_GRAPH: u32 = 7;
const GRAPH_NODE: u32 = 1;
const GRAPH_INITIALIZER: u32 = 5;
const NODE_INPUT: u32 = 1;
const NODE_OUTPUT: u32 = 2;
const NODE_NAME: u32 = 3;
const NODE_OP_TYPE: u32 = 4;
const NODE_ATTRIBUTE: u32 = 5;
const ATTRIBUTE_NAME: u32 = 1;
const ATTRIBUTE_I: u32 = 3;
const ATTRIBUTE_S: u32 = 4;
const ATTRIBUTE_INTS: u32 = 8;
const ATTRIBUTE_TYPE: u32 = 20;
const ATTRIBUTE_TYPE_INT: i64 = 2;
const ATTRIBUTE_TYPE_STRING: i64 = 3;
const ATTRIBUTE_TYPE_INTS: i64 = 7;
const TENSOR_DIMS: u32 = 1;
const TENSOR_DATA_TYPE: u32 = 2;
const TENSOR_NAME: u32 = 8;
const TENSOR_RAW_DATA: u32 = 9;
const FLOAT: u64 = 1;

#[derive(Clone, Copy)]
enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed,
}

struct Field<'a> {
    number: u32,
    value: Value<'a>,
    raw: &'a [u8],
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *buf.get(*pos).ok_or_else(|| anyhow!("truncated varint"))?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("varint is too long")
}

fn fields(buf: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < buf.len() {
        let start = pos;
        let key = read_varint(buf, &mut pos)?;
        let value = match key & 7 {
            0 => Value::Varint(read_varint(buf, &mut pos)?),
            1 | 5 => {
                pos += if key & 7 == 1 { 8 } else { 4 };
                ensure!(pos <= buf.len(), "truncated fixed-size field");
                Value::Fixed
            }
            2 => {
                let len = read_varint(buf, &mut pos)? as usize;
                let bytes = buf
                    .get(pos..pos + len)
                    .ok_or_else(|| anyhow!("truncated length-delimited field"))?;
                pos += len;
                Value::Bytes(bytes)
            }
            wire_type => bail!("unsupported protobuf wire type {}", wire_type),
        };
        out.push(Field {
            number: (key >> 3) as u32,
            value,
            raw: &buf[start..pos],
        });
    }
    Ok(out)
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_bytes(out: &mut Vec<u8>, number: u32, bytes: &[u8]) {
    put_varint(out, u64::from(number) << 3 | 2);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn put_int(out: &mut Vec<u8>, number: u32, value: i64) {
    put_varint(out, u64::from(number) << 3);
    put_varint(out, value as u64);
}

fn bytes_fields<'a>(fields: &[Field<'a>], number: u32) -> impl Iterator<Item = &'a [u8]> {
    fields
        .iter()
        .filter(move |field| field.number == number)
        .filter_map(|field| match field.value {
            Value::Bytes(bytes) => Some(bytes),
            _ => None,
        })
}

fn string(fields: &[Field], number: u32) -> Result<String> {
    let bytes = bytes_fields(fields, number).next().unwrap_or_default();
    Ok(std::str::from_utf8(bytes)?.to_owned())
}

fn strings(fields: &[Field], number: u32) -> Result<Vec<String>> {
    bytes_fields(fields, number)
        .map(|bytes| Ok(std::str::from_utf8(bytes)?.to_owned()))
        .collect()
}

fn varint(fields: &[Field], number: u32) -> Option<u64> {
    fields
        .iter()
        .filter(|field| field.number == number)
        .find_map(|field| match field.value {
            Value::Varint(value) => Some(value),
            _ => None,
        })
}

fn ints(fields: &[Field], number: u32) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for field in fields.iter().filter(|field| field.number == number) {
        match field.value {
            Value::Varint(value) => out.push(value as i64),
            Value::Bytes(packed) => {
                let mut pos = 0;
                while pos < packed.len() {
                    out.push(read_varint(packed, &mut pos)? as i64);
                }
            }
            Value::Fixed => bail!("unexpected fixed-size integer"),
        }
    }
    Ok(out)
}

enum Attribute {
    Int(i64),
    Ints(Vec<i64>),
    String(Vec<u8>),
    Other,
}

fn ints_attribute(name: &str, values: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    put_bytes(&mut out, ATTRIBUTE_NAME, name.as_bytes());
    for &value in values {
        put_int(&mut out, ATTRIBUTE_INTS, value);
    }
    put_int(&mut out, ATTRIBUTE_TYPE, ATTRIBUTE_TYPE_INTS);
    out
}

struct Node {
    inputs: Vec<String>,
    outputs: Vec<String>,
    op_type: String,
    rest: Vec<u8>,
}

impl Node {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let fields = fields(bytes)?;
        Ok(Self {
            inputs: strings(&fields, NODE_INPUT)?,
            outputs: strings(&fields, NODE_OUTPUT)?,
            op_type: string(&fields, NODE_OP_TYPE)?,
            rest: fields
                .iter()
                .filter(|field| field.number != NODE_INPUT && field.number != NODE_OUTPUT)
                .flat_map(|field| field.raw)
                .copied()
                .collect(),
        })
    }

    fn new(
        name: &str,
        op_type: &str,
        inputs: Vec<String>,
        outputs: Vec<String>,
        attributes: &[Vec<u8>],
    ) -> Self {
        let mut rest = Vec::new();
        put_bytes(&mut rest, NODE_NAME, name.as_bytes());
        put_bytes(&mut rest, NODE_OP_TYPE, op_type.as_bytes());
        for attribute in attributes {
            put_bytes(&mut rest, NODE_ATTRIBUTE, attribute);
        }
        Self {
            inputs,
            outputs,
            op_type: op_type.to_owned(),
            rest,
        }
    }

    fn name(&self) -> Result<String> {
        string(&fields(&self.rest)?, NODE_NAME)
    }

    fn attributes(&self) -> Result<HashMap<String, Attribute>> {
        let fields = fields(&self.rest)?;
        bytes_fields(&fields, NODE_ATTRIBUTE)
            .map(|bytes| {
                let attribute = self::fields(bytes)?;
                let name = string(&attribute, ATTRIBUTE_NAME)?;
                let value = match varint(&attribute, ATTRIBUTE_TYPE).map(|t| t as i64) {
                    Some(ATTRIBUTE_TYPE_INT) => {
                        Attribute::Int(varint(&attribute, ATTRIBUTE_I).unwrap_or(0) as i64)
                    }
                    Some(ATTRIBUTE_TYPE_INTS) => Attribute::Ints(ints(&attribute, ATTRIBUTE_INTS)?),
                    Some(ATTRIBUTE_TYPE_STRING) => Attribute::String(
                        bytes_fields(&attribute, ATTRIBUTE_S)
                            .next()
                            .unwrap_or_default()
                            .to_vec(),
                    ),
                    _ => Attribute::Other,
                };
                Ok((name, value))
            })
            .collect()
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let mut node = Vec::new();
        for input in &self.inputs {
            put_bytes(&mut node, NODE_INPUT, input.as_bytes());
        }
        for output in &self.outputs {
            put_bytes(&mut node, NODE_OUTPUT, output.as_bytes());
        }
        node.extend_from_slice(&self.rest);
        put_bytes(out, GRAPH_NODE, &node);
    }
}

struct Initializer<'a> {
    name: String,
    dims: Vec<i64>,
    data_type: u64,
    raw_data: Option<Vec<u8>>,
    original: &'a [u8],
    other: Vec<u8>,
    changed: bool,
}

impl<'a> Initializer<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let fields = fields(bytes)?;
        Ok(Self {
            name: string(&fields, TENSOR_NAME)?,
            dims: ints(&fields, TENSOR_DIMS)?,
            data_type: varint(&fields, TENSOR_DATA_TYPE).unwrap_or(0),
            raw_data: bytes_fields(&fields, TENSOR_RAW_DATA)
                .next()
                .map(<[u8]>::to_vec),
            original: bytes,
            other: fields
                .iter()
                .filter(|field| {
                    ![TENSOR_DIMS, TENSOR_DATA_TYPE, TENSOR_RAW_DATA].contains(&field.number)
                })
                .flat_map(|field| field.raw)
                .copied()
                .collect(),
            changed: false,
        })
    }

    fn floats(&self) -> Result<Vec<f32>> {
        ensure!(
            self.data_type == FLOAT,
            "initializer {} is not float32",
            self.name
        );
        let raw = self
            .raw_data
            .as_ref()
            .ok_or_else(|| anyhow!("initializer {} has no raw data", self.name))?;
        Ok(raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&bytes| f32::from_le_bytes(bytes))
            .collect())
    }

    fn set_floats(&mut self, dims: Vec<i64>, values: &[f32]) {
        self.dims = dims;
        self.raw_data = Some(
            values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect(),
        );
        self.changed = true;
    }

    fn encode(&self, out: &mut Vec<u8>) {
        if !self.changed {
            put_bytes(out, GRAPH_INITIALIZER, self.original);
            return;
        }
        let mut tensor = self.other.clone();
        for &dim in &self.dims {
            put_int(&mut tensor, TENSOR_DIMS, dim);
        }
        put_int(&mut tensor, TENSOR_DATA_TYPE, self.data_type as i64);
        if let Some(raw) = &self.raw_data {
            put_bytes(&mut tensor, TENSOR_RAW_DATA, raw);
        }
        put_bytes(out, GRAPH_INITIALIZER, &tensor);
    }
}

fn ints_or(attributes: &HashMap<String, Attribute>, name: &str, default: &[i64]) -> Vec<i64> {
    match attributes.get(name) {
        Some(Attribute::Ints(values)) => values.clone(),
        _ => default.to_vec(),
    }
}

fn conv_transpose_as_conv(
    node: &Node,
    initializers: &mut [Initializer],
    index: &HashMap<String, usize>,
    uses: &HashMap<&str, usize>,
) -> Result<Option<Vec<Node>>> {
    let attributes = node.attributes()?;
    let single_use = |name: &str| {
        index
            .get(name)
            .copied()
            .filter(|_| uses.get(name) == Some(&1))
    };
    let Some(weight) = node.inputs.get(1).and_then(|name| single_use(name)) else {
        return Ok(None);
    };
    let [c_in, c_out, kh, kw] = initializers[weight].dims[..] else {
        return Ok(None);
    };
    let (c_in, c_out, kh, kw) = (c_in as usize, c_out as usize, kh as usize, kw as usize);
    let plain = matches!(attributes.get("group"), None | Some(Attribute::Int(1)))
        && ints_or(&attributes, "dilations", &[1, 1]) == [1, 1]
        && ints_or(&attributes, "output_padding", &[0, 0]) == [0, 0]
        && !attributes.contains_key("output_shape")
        && match attributes.get("auto_pad") {
            None => true,
            Some(Attribute::String(mode)) => mode == b"NOTSET",
            Some(_) => false,
        };
    if !plain || initializers[weight].data_type != FLOAT {
        return Ok(None);
    }
    let strides = ints_or(&attributes, "strides", &[1, 1]);
    let pads = ints_or(&attributes, "pads", &[0, 0, 0, 0]);
    if strides != [1, 1] || pads.len() != 4 {
        return Ok(None);
    }
    let kernel = [kh as i64, kw as i64, kh as i64, kw as i64];
    if pads
        .iter()
        .zip(kernel)
        .any(|(&pad, k)| pad < 0 || pad > k - 1)
    {
        return Ok(None);
    }
    let w = initializers[weight].floats()?;
    let name = node.name()?;
    let mut flipped = vec![0.0f32; w.len()];
    for ci in 0..c_in {
        for co in 0..c_out {
            for y in 0..kh {
                for x in 0..kw {
                    flipped[((co * c_in + ci) * kh + y) * kw + x] =
                        w[((ci * c_out + co) * kh + (kh - 1 - y)) * kw + (kw - 1 - x)];
                }
            }
        }
    }
    let dims = vec![c_out as i64, c_in as i64, kh as i64, kw as i64];
    initializers[weight].set_floats(dims, &flipped);
    let conv_pads: Vec<i64> = pads
        .iter()
        .zip(kernel)
        .map(|(&pad, k)| k - 1 - pad)
        .collect();
    Ok(Some(vec![Node::new(
        &name,
        "Conv",
        node.inputs.clone(),
        node.outputs.clone(),
        &[
            ints_attribute("kernel_shape", &[kh as i64, kw as i64]),
            ints_attribute("pads", &conv_pads),
            ints_attribute("strides", &[1, 1]),
        ],
    )]))
}

fn rewrite(model: &[u8]) -> Result<Option<(Vec<u8>, usize)>> {
    let model_fields = fields(model)?;
    let graph_bytes = bytes_fields(&model_fields, MODEL_GRAPH)
        .next()
        .ok_or_else(|| anyhow!("model has no graph"))?;
    let graph = fields(graph_bytes)?;

    let parsed = bytes_fields(&graph, GRAPH_NODE)
        .map(Node::parse)
        .collect::<Result<Vec<_>>>()?;
    if !parsed.iter().any(|node| node.op_type == "ConvTranspose") {
        return Ok(None);
    }
    let mut initializers = bytes_fields(&graph, GRAPH_INITIALIZER)
        .map(Initializer::parse)
        .collect::<Result<Vec<_>>>()?;
    let index: HashMap<String, usize> = initializers
        .iter()
        .enumerate()
        .map(|(i, initializer)| (initializer.name.clone(), i))
        .collect();

    let mut uses: HashMap<&str, usize> = HashMap::new();
    for input in parsed.iter().flat_map(|node| &node.inputs) {
        *uses.entry(input.as_str()).or_default() += 1;
    }
    let mut nodes = Vec::with_capacity(parsed.len());
    let mut converted = 0;
    for node in &parsed {
        let replacement = if node.op_type == "ConvTranspose" {
            conv_transpose_as_conv(node, &mut initializers, &index, &uses)?
        } else {
            None
        };
        match replacement {
            Some(replacement) => {
                converted += 1;
                nodes.extend(replacement);
            }
            None => nodes.push(Node {
                inputs: node.inputs.clone(),
                outputs: node.outputs.clone(),
                op_type: node.op_type.clone(),
                rest: node.rest.clone(),
            }),
        }
    }

    if converted == 0 {
        return Ok(None);
    }

    let mut new_graph = Vec::with_capacity(graph_bytes.len());
    let mut nodes_written = false;
    let mut initializers_written = false;
    for field in &graph {
        match field.number {
            GRAPH_NODE if !nodes_written => {
                nodes.iter().for_each(|node| node.encode(&mut new_graph));
                nodes_written = true;
            }
            GRAPH_INITIALIZER if !initializers_written => {
                initializers
                    .iter()
                    .for_each(|initializer| initializer.encode(&mut new_graph));
                initializers_written = true;
            }
            GRAPH_NODE | GRAPH_INITIALIZER => {}
            _ => new_graph.extend_from_slice(field.raw),
        }
    }

    let mut out = Vec::with_capacity(model.len());
    for field in &model_fields {
        if field.number == MODEL_GRAPH {
            put_bytes(&mut out, MODEL_GRAPH, &new_graph);
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    Ok(Some((out, converted)))
}

pub fn optimized_model(model_path: &Path, use_gpu: bool) -> Option<Vec<u8>> {
    if use_gpu || !cfg!(target_arch = "x86_64") {
        return None;
    }
    match fs::read(model_path)
        .map_err(anyhow::Error::from)
        .and_then(|model| rewrite(&model))
    {
        Ok(Some((model, converted))) => {
            log::info!(
                "Rewrote {} ConvTranspose layers of {} as Conv",
                converted,
                model_path.display()
            );
            Some(model)
        }
        Ok(None) => None,
        Err(e) => {
            log::warn!(
                "Could not optimize {}, using it as it is: {}",
                model_path.display(),
                e
            );
            None
        }
    }
}
