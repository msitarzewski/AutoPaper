//! Conservative conversion of ComfyUI's official UI templates to API graphs. This is deliberately not a
//! general LiteGraph interpreter: only understood local text-to-image paths become executable workflows.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use super::{Mapped, NodeMap, OutputNode, Placeholders, Requires, Slot};

type Result<T> = std::result::Result<T, String>;

struct Scope<'a> {
    graph: &'a Value,
    prefix: String,
    bindings: BTreeMap<String, Value>,
    overrides: BTreeMap<(String, String), Value>,
}

struct Converter<'a> {
    definitions: BTreeMap<String, &'a Value>,
    server: &'a Map<String, Value>,
    graph: Map<String, Value>,
    visiting: BTreeSet<String>,
}

#[derive(Clone)]
struct Link {
    origin: String,
    slot: u64,
    target: String,
    target_slot: u64,
}

/// Keep the template's model, encoders, VAE, sampling and negative prompt. Bind only the wallpaper prompt,
/// dimensions and seed. Unsupported or ambiguous templates are unavailable, never guessed into a menu.
pub(super) fn adapt(template: &Value, name: &str, server: &Map<String, Value>) -> Result<Mapped> {
    let definitions_list = template
        .pointer("/definitions/subgraphs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|definition| Ok((string(definition, "id")?.to_owned(), definition)))
        .collect::<Result<Vec<_>>>()?;
    let definitions: BTreeMap<_, _> = definitions_list.iter().cloned().collect();
    if definitions.len() != definitions_list.len() || definitions.len() > 32 {
        return Err("duplicate or excessive subgraph definitions".into());
    }
    let mut converter = Converter { definitions, server, graph: Map::new(), visiting: BTreeSet::new() };
    let scope = Scope { graph: template, prefix: "t".into(), bindings: BTreeMap::new(), overrides: BTreeMap::new() };
    let saves: Vec<_> = nodes(template)?
        .iter()
        .filter(|node| matches!(node.get("type").and_then(Value::as_str), Some("SaveImage" | "SaveImageAdvanced")))
        .filter(|node| node.get("mode").and_then(Value::as_u64).unwrap_or(0) == 0)
        .filter(|node| {
            node.get("inputs").and_then(Value::as_array).is_some_and(|inputs| {
                inputs.iter().any(|input| {
                    input.get("name").and_then(Value::as_str) == Some("images")
                        && input.get("link").is_some_and(|link| !link.is_null())
                })
            })
        })
        .collect();
    if saves.len() != 1 {
        return Err("the template must have one unambiguous saved image".into());
    }
    let images = converter.input(&scope, saves[0], "images", None)?.ok_or("the saved image is disconnected")?;
    if !is_connection(&images) {
        return Err("the template has no generated image".into());
    }
    converter.validate_connection(&images, &json!(["IMAGE"]))?;
    if !server.contains_key("SaveImage") {
        return Err("SaveImage is unavailable".into());
    }
    converter.graph.insert(
        "autopaper_save".into(),
        json!({
            "class_type": "SaveImage", "inputs": {"images": images, "filename_prefix": "AutoPaper"}
        }),
    );

    let mut prompts = BTreeSet::new();
    let mut latents = Vec::new();
    let mut seeds = Vec::new();
    let mut models = 0;
    let mut requires = Requires::default();
    for (id, node) in &converter.graph {
        let class = string(node, "class_type")?;
        let inputs = node["inputs"].as_object().ok_or("node has no inputs")?;
        if latent(class) {
            latents.push(id.clone());
        }
        let seed = match class {
            "KSampler" => Some("seed"),
            "KSamplerAdvanced" | "RandomNoise" => Some("noise_seed"),
            _ => None,
        };
        if let Some(input) = seed {
            if !inputs.get(input).is_some_and(Value::is_u64) {
                return Err("the template's generation seed is not a scalar".into());
            }
            seeds.push(Slot { node: id.clone(), input: input.into() });
        }
        for input in match class {
            "KSampler" | "KSamplerAdvanced" | "CFGGuider" => &["positive"][..],
            "BasicGuider" => &["conditioning"][..],
            _ => &[],
        } {
            let value = inputs.get(*input).ok_or("missing positive conditioning")?;
            positive_prompts(&converter.graph, value, &mut prompts, &mut BTreeSet::new())?;
        }
        if matches!(class, "UNETLoader" | "CheckpointLoaderSimple") {
            models += 1;
        }
        let requirement = match class {
            "UNETLoader" => Some((&mut requires.diffusion_models, "unet_name")),
            "CLIPLoader" => Some((&mut requires.text_encoders, "clip_name")),
            "VAELoader" => Some((&mut requires.vae, "vae_name")),
            _ => None,
        };
        if let Some((files, input)) = requirement {
            files.push(inputs.get(input).and_then(Value::as_str).ok_or("model file is not a literal")?.to_owned());
        }
    }
    if models != 1 || prompts.len() != 1 || latents.len() != 1 || seeds.len() != 1 {
        return Err("the template needs one model, positive prompt, empty latent and generation seed".into());
    }
    let (prompt_node, prompt_input) = prompts.into_iter().next().unwrap();
    let mapped = Mapped {
        graph: converter.graph,
        map: NodeMap {
            name: name.to_owned(),
            placeholders: Placeholders {
                prompt: Slot { node: prompt_node, input: prompt_input },
                width: Slot { node: latents[0].clone(), input: "width".into() },
                height: Slot { node: latents[0].clone(), input: "height".into() },
                seed: seeds.remove(0),
            },
            output: Some(OutputNode { node: "autopaper_save".into() }),
            requires,
        },
    };
    super::bind_available_files(&mapped, server)
        .ok_or_else(|| "the template's required files or node choices are unavailable".into())
}

impl Converter<'_> {
    fn output(&mut self, scope: &Scope<'_>, id: &str, slot: u64) -> Result<Option<Value>> {
        let input_id = scope.graph.pointer("/inputNode/id").and_then(node_id);
        if input_id.as_deref() == Some(id) {
            let input = scope.graph["inputs"].get(slot as usize).ok_or("unknown subgraph input")?;
            return Ok(scope.bindings.get(string(input, "name")?).cloned());
        }
        let node = nodes(scope.graph)?
            .iter()
            .find(|node| node.get("id").and_then(node_id).as_deref() == Some(id))
            .ok_or("link references an absent node")?;
        if node.get("mode").and_then(Value::as_u64).unwrap_or(0) != 0 {
            return Err("muted or bypassed nodes are unsupported".into());
        }
        let key = format!("{}_{id}", scope.prefix);
        if self.visiting.len() >= 32 || self.graph.len() >= 256 || !self.visiting.insert(key.clone()) {
            return Err("recursive or excessively deep template".into());
        }
        let result = self.output_inner(scope, node, slot, &key);
        self.visiting.remove(&key);
        result
    }

    fn output_inner(&mut self, scope: &Scope<'_>, node: &Value, slot: u64, key: &str) -> Result<Option<Value>> {
        let class = string(node, "type")?;
        if let Some(definition) = self.definitions.get(class).copied() {
            let child = self.child(scope, node, definition, key)?;
            let output_id =
                definition.pointer("/outputNode/id").and_then(node_id).ok_or("subgraph has no output node")?;
            let matches = links(definition)?
                .into_iter()
                .filter(|link| link.target == output_id && link.target_slot == slot)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err("ambiguous subgraph output".into());
            }
            return self.output(&child, &matches[0].origin, matches[0].slot);
        }
        // These are UI routing primitives. Follow only the chosen branch, so unused prompt enhancers,
        // preview outputs and their model downloads do not become wallpaper dependencies.
        match class {
            "PreviewAny" | "Reroute" if slot == 0 => {
                let input = if class == "PreviewAny" { "source" } else { "" };
                return self.input(scope, node, input, None);
            }
            "PrimitiveStringMultiline" | "PrimitiveString" | "PrimitiveInt" | "PrimitiveFloat" | "PrimitiveBoolean"
                if slot == 0 =>
            {
                let defaults = primitive_value(node)?;
                return self.input(scope, node, "value", defaults.as_ref());
            }
            "ComfySwitchNode" if slot == 0 => {
                let defaults = primitive_value(node)?;
                let switch = self
                    .input(scope, node, "switch", defaults.as_ref())?
                    .and_then(|value| value.as_bool())
                    .ok_or("dynamic switches are unsupported")?;
                return self.input(scope, node, if switch { "on_true" } else { "on_false" }, None);
            }
            _ => {}
        }
        if !supported(class) {
            return Err(format!("unsupported text-to-image node {class}"));
        }
        let schema = self.server.get(class).ok_or_else(|| format!("missing node {class}"))?;
        if schema.get("api_node").and_then(Value::as_bool) == Some(true) {
            return Err("API nodes are not local text-to-image workflows".into());
        }
        if schema.get("output").and_then(Value::as_array).and_then(|outputs| outputs.get(slot as usize)).is_none() {
            return Err("unknown node output slot".into());
        }
        if self.graph.contains_key(key) {
            return Ok(Some(json!([key, slot])));
        }
        let defaults = widget_values(node, schema)?;
        let mut inputs = Map::new();
        for group in ["required", "optional"] {
            let Some(specs) = schema.pointer(&format!("/input/{group}")).and_then(Value::as_object) else {
                continue;
            };
            for (name, spec) in specs {
                // The wallpaper renderer owns dimensions; ignore UI-only aspect-ratio selectors feeding them.
                let value = if latent(class) && name == "batch_size" {
                    Some(json!(1))
                } else if latent(class) && matches!(name.as_str(), "width" | "height") {
                    Some(self.dimension(scope, node, name, defaults.get(name))?)
                } else {
                    self.input(scope, node, name, defaults.get(name))?
                };
                if let Some(value) = value {
                    validate_value(&value, spec)?;
                    self.validate_connection(&value, spec)?;
                    inputs.insert(name.clone(), value);
                } else if group == "required" && !empty_autogrow(spec) {
                    return Err(format!("{class} has no value for {name}"));
                }
            }
        }
        // Refuse connected dynamic inputs that the server schema cannot describe.
        for input in node.get("inputs").and_then(Value::as_array).into_iter().flatten() {
            if input.get("link").is_some_and(|link| !link.is_null()) && !inputs.contains_key(string(input, "name")?) {
                return Err(format!("unsupported connected input on {class}"));
            }
        }
        self.graph.insert(key.to_owned(), json!({"class_type": class, "inputs": inputs}));
        Ok(Some(json!([key, slot])))
    }

    fn input(&mut self, scope: &Scope<'_>, node: &Value, name: &str, default: Option<&Value>) -> Result<Option<Value>> {
        let id = node.get("id").and_then(node_id).ok_or("node has no id")?;
        if let Some(value) = scope.overrides.get(&(id, name.to_owned())) {
            return Ok(Some(value.clone()));
        }
        let input = node.get("inputs").and_then(Value::as_array).and_then(|inputs| {
            inputs.iter().enumerate().find(|(_, input)| input.get("name").and_then(Value::as_str) == Some(name))
        });
        if let Some((index, link_id)) = input
            .and_then(|(index, input)| input.get("link").map(|link| (index, link)))
            .filter(|(_, value)| !value.is_null())
        {
            let link = find_link(scope.graph, link_id)?;
            if link.target != node.get("id").and_then(node_id).unwrap() || link.target_slot != index as u64 {
                return Err("link target does not match its input".into());
            }
            if let Some(value) = self.output(scope, &link.origin, link.slot)? {
                return Ok(Some(value));
            }
        }
        Ok(default.cloned())
    }

    fn validate_connection(&self, value: &Value, spec: &Value) -> Result<()> {
        if !is_connection(value) {
            return Ok(());
        }
        let node = self.graph.get(value[0].as_str().unwrap()).ok_or("link source was not converted")?;
        let schema = self.server.get(string(node, "class_type")?).ok_or("link source has no schema")?;
        let output = schema["output"]
            .get(value[1].as_u64().unwrap() as usize)
            .and_then(Value::as_str)
            .ok_or("link source has no output type")?;
        if spec.get(0).and_then(Value::as_str) != Some(output) {
            return Err("connection types do not match".into());
        }
        Ok(())
    }

    fn dimension(&self, scope: &Scope<'_>, node: &Value, name: &str, default: Option<&Value>) -> Result<Value> {
        let id = node.get("id").and_then(node_id).ok_or("node has no id")?;
        let mut value = scope.overrides.get(&(id, name.to_owned())).or(default);
        let input = node
            .get("inputs")
            .and_then(Value::as_array)
            .and_then(|inputs| inputs.iter().find(|input| input.get("name").and_then(Value::as_str) == Some(name)));
        if let Some(link_id) = input.and_then(|input| input.get("link")).filter(|value| !value.is_null()) {
            let link = find_link(scope.graph, link_id)?;
            if scope.graph.pointer("/inputNode/id").and_then(node_id).as_deref() == Some(&link.origin) {
                let exposed = scope.graph["inputs"].get(link.slot as usize).ok_or("unknown dimension input")?;
                value = scope.bindings.get(string(exposed, "name")?).or(value);
            }
        }
        value
            .filter(|value| value.as_u64().is_some_and(|value| value > 0))
            .cloned()
            .ok_or_else(|| "the template's image dimensions have no declared scalar default".into())
    }

    fn child<'a>(&mut self, parent: &Scope<'_>, node: &Value, definition: &'a Value, key: &str) -> Result<Scope<'a>> {
        let mut bindings = BTreeMap::new();
        let mut overrides = BTreeMap::new();
        let named = node.get("widgets_values_named").and_then(Value::as_object);
        let positional = node.get("widgets_values").and_then(Value::as_array).filter(|values| !values.is_empty());
        if named.is_none()
            && let Some(values) = positional
        {
            let proxies = node
                .pointer("/properties/proxyWidgets")
                .and_then(Value::as_array)
                .ok_or("subgraph widget order is unknown")?;
            if proxies.len() != values.len() {
                return Err("subgraph widget values do not match their proxy map".into());
            }
            for (proxy, value) in proxies.iter().zip(values) {
                let target = proxy.get(0).and_then(node_id).ok_or("invalid proxy target")?;
                let input = proxy.get(1).and_then(Value::as_str).ok_or("invalid proxy input")?;
                if input != "control_after_generate" {
                    overrides.insert((target, input.to_owned()), value.clone());
                }
            }
        }
        for input in definition.get("inputs").and_then(Value::as_array).into_iter().flatten() {
            let name = string(input, "name")?;
            let label = input.get("label").and_then(Value::as_str).unwrap_or(name);
            if matches!(label, "enable_prompt_upsampler" | "refine_prompt")
                && input.get("type").and_then(Value::as_str) == Some("BOOLEAN")
            {
                bindings.insert(name.to_owned(), json!(false));
            } else if matches!(name, "width" | "height") {
                // The empty-latent node receives the final display dimensions later; do not evaluate a
                // resolution helper while entering the subgraph either.
                if let Some(value) = named.and_then(|values| values.get(name)).filter(|value| value.is_u64()) {
                    bindings.insert(name.to_owned(), value.clone());
                }
            } else if let Some(value) = self.input(parent, node, name, named.and_then(|values| values.get(name)))? {
                bindings.insert(name.to_owned(), value);
            }
        }
        Ok(Scope { graph: definition, prefix: key.to_owned(), bindings, overrides })
    }
}

fn positive_prompts(
    graph: &Map<String, Value>,
    value: &Value,
    found: &mut BTreeSet<(String, String)>,
    visited: &mut BTreeSet<String>,
) -> Result<()> {
    let id = value.get(0).and_then(Value::as_str).ok_or("positive conditioning is not connected")?;
    if !visited.insert(id.to_owned()) {
        return Ok(());
    }
    let node = graph.get(id).ok_or("positive conditioning node is absent")?;
    let class = string(node, "class_type")?;
    let input = match class {
        "CLIPTextEncode" => Some("text"),
        "TextEncodeQwenImage21" => Some("prompt"),
        _ => None,
    };
    if let Some(input) = input {
        if !node["inputs"].get(input).is_some_and(Value::is_string) {
            return Err("the positive prompt is not a plain string".into());
        }
        found.insert((id.to_owned(), input.to_owned()));
    } else if matches!(class, "ConditioningZeroOut" | "ConditioningSetTimestepRange" | "FluxGuidance") {
        positive_prompts(graph, &node["inputs"]["conditioning"], found, visited)?;
    } else {
        return Err(format!("unsupported positive conditioning {class}"));
    }
    Ok(())
}

fn widget_values(node: &Value, schema: &Value) -> Result<Map<String, Value>> {
    if let Some(named) = node.get("widgets_values_named").and_then(Value::as_object) {
        return Ok(named.clone());
    }
    let empty = Vec::new();
    let values = node.get("widgets_values").and_then(Value::as_array).unwrap_or(&empty);
    if values.is_empty() {
        return Ok(Map::new());
    }
    let mut result = Map::new();
    let mut cursor = 0;
    for group in ["required", "optional"] {
        if group == "optional" && cursor == values.len() {
            break;
        }
        let Some(specs) = schema.pointer(&format!("/input/{group}")).and_then(Value::as_object) else {
            continue;
        };
        let order = schema
            .pointer(&format!("/input_order/{group}"))
            .and_then(Value::as_array)
            .ok_or("positional widgets require the server's input order")?;
        if order.len() != specs.len() {
            return Err("incomplete server input order".into());
        }
        if order.iter().filter_map(Value::as_str).collect::<BTreeSet<_>>().len() != specs.len() {
            return Err("ambiguous server input order".into());
        }
        for name in order {
            let name = name.as_str().ok_or("invalid server input order")?;
            let spec = specs.get(name).ok_or("unknown ordered input")?;
            let kind = spec.get(0).ok_or("invalid input spec")?;
            if (kind.is_array() || matches!(kind.as_str(), Some("STRING" | "INT" | "FLOAT" | "BOOLEAN" | "COMBO")))
                && spec.pointer("/1/forceInput").and_then(Value::as_bool) != Some(true)
            {
                let Some(value) = values.get(cursor) else {
                    if group == "optional" {
                        continue;
                    }
                    return Err("missing positional widget value".into());
                };
                result.insert(name.to_owned(), value.clone());
                cursor += 1;
                if spec.pointer("/1/control_after_generate").and_then(Value::as_bool) == Some(true) {
                    if !matches!(
                        values.get(cursor).and_then(Value::as_str),
                        Some("fixed" | "increment" | "decrement" | "randomize")
                    ) {
                        return Err("missing seed control widget".into());
                    }
                    cursor += 1;
                }
            }
        }
    }
    if cursor != values.len() {
        return Err("unmapped positional widget values".into());
    }
    Ok(result)
}

fn validate_value(value: &Value, spec: &Value) -> Result<()> {
    if is_connection(value) {
        return Ok(());
    }
    let kind = spec.get(0).ok_or("missing input type")?;
    let valid = if kind.is_array() {
        value.is_string()
    } else {
        match kind.as_str() {
            Some("STRING" | "COMBO") => value.is_string(),
            Some("INT") => value.is_i64() || value.is_u64(),
            Some("FLOAT") => value.is_number(),
            Some("BOOLEAN") => value.is_boolean(),
            _ => false,
        }
    };
    if valid { Ok(()) } else { Err("widget value does not match its input type".into()) }
}

fn empty_autogrow(spec: &Value) -> bool {
    spec.get(0).and_then(Value::as_str) == Some("COMFY_AUTOGROW_V3")
        && spec.pointer("/1/template/min").and_then(Value::as_u64) == Some(0)
}

fn primitive_value(node: &Value) -> Result<Option<Value>> {
    let input = if node.get("type").and_then(Value::as_str) == Some("ComfySwitchNode") { "switch" } else { "value" };
    if let Some(value) = node.get("widgets_values_named").and_then(|values| values.get(input)) {
        return Ok(Some(value.clone()));
    }
    let values = node.get("widgets_values").and_then(Value::as_array).ok_or("primitive has no widget values")?;
    if values.len() != 1 {
        return Err("ambiguous primitive widget values".into());
    }
    Ok(values.first().cloned())
}

fn latent(class: &str) -> bool {
    matches!(class, "EmptyLatentImage" | "EmptySD3LatentImage" | "EmptyFlux2LatentImage")
}

fn supported(class: &str) -> bool {
    matches!(
        class,
        "UNETLoader"
            | "CheckpointLoaderSimple"
            | "CLIPLoader"
            | "DualCLIPLoader"
            | "TripleCLIPLoader"
            | "VAELoader"
            | "VAEDecode"
            | "CLIPTextEncode"
            | "TextEncodeQwenImage21"
            | "EmptyLatentImage"
            | "EmptySD3LatentImage"
            | "EmptyFlux2LatentImage"
            | "KSampler"
            | "KSamplerAdvanced"
            | "SamplerCustomAdvanced"
            | "RandomNoise"
            | "KSamplerSelect"
            | "CFGGuider"
            | "BasicGuider"
            | "BasicScheduler"
            | "ManualSigmas"
            | "ConditioningZeroOut"
            | "ConditioningSetTimestepRange"
            | "FluxGuidance"
            | "ModelSamplingAuraFlow"
            | "ModelSamplingSD3"
            | "ModelSamplingDiscrete"
            | "ModelSamplingContinuousEDM"
            | "QwenImage21Cache"
            | "SkipLayerGuidanceDiTSimple"
            | "LoraLoader"
            | "LoraLoaderModelOnly"
    )
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing {key}"))
}

fn node_id(value: &Value) -> Option<String> {
    value.as_i64().or_else(|| value.as_str()?.parse::<i64>().ok()).map(|id| id.to_string())
}

fn nodes(graph: &Value) -> Result<&Vec<Value>> {
    let nodes = graph.get("nodes").and_then(Value::as_array).ok_or("template has no UI nodes")?;
    let ids = nodes
        .iter()
        .map(|node| node.get("id").and_then(node_id).ok_or("node has no id"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    if nodes.len() > 256 || ids.len() != nodes.len() {
        return Err("duplicate or excessive template nodes".into());
    }
    for node in nodes {
        if let Some(inputs) = node.get("inputs").and_then(Value::as_array) {
            let names = inputs.iter().map(|input| string(input, "name")).collect::<Result<BTreeSet<_>>>()?;
            if names.len() != inputs.len() {
                return Err("duplicate node input names".into());
            }
        }
    }
    Ok(nodes)
}

fn is_connection(value: &Value) -> bool {
    value.as_array().is_some_and(|parts| parts.len() == 2 && parts[0].is_string() && parts[1].is_u64())
}

fn parse_link(value: &Value) -> Result<Link> {
    let part = |index: usize, field: &str| {
        if value.is_array() { value.get(index) } else { value.get(field) }
    };
    Ok(Link {
        origin: part(1, "origin_id").and_then(node_id).ok_or("link has no origin")?,
        slot: part(2, "origin_slot").and_then(Value::as_u64).ok_or("link has no origin slot")?,
        target: part(3, "target_id").and_then(node_id).ok_or("link has no target")?,
        target_slot: part(4, "target_slot").and_then(Value::as_u64).ok_or("link has no target slot")?,
    })
}

fn links(graph: &Value) -> Result<Vec<Link>> {
    let links = graph.get("links").and_then(Value::as_array).ok_or("graph has no links")?;
    if links.len() > 1024 {
        return Err("excessive template links".into());
    }
    links.iter().map(parse_link).collect()
}

fn find_link(graph: &Value, id: &Value) -> Result<Link> {
    if graph.get("links").and_then(Value::as_array).is_some_and(|links| links.len() > 1024) {
        return Err("excessive template links".into());
    }
    let matches: Vec<_> = graph
        .get("links")
        .and_then(Value::as_array)
        .ok_or("graph has no links")?
        .iter()
        .filter(|link| (if link.is_array() { link.get(0) } else { link.get("id") }) == Some(id))
        .collect();
    if matches.len() != 1 {
        return Err("missing or duplicate link id".into());
    }
    parse_link(matches[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../../tests/fixtures/comfyui/official-templates.response.json")).unwrap()
    }

    fn server() -> Map<String, Value> {
        let mut server: Map<String, Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/comfyui/object_info.response.json")).unwrap();
        server.extend(fixtures()["server_additions"].as_object().unwrap().clone());
        server
    }

    fn template(name: &str) -> Value {
        fixtures()["templates"][name].clone()
    }

    fn child_node(template: &mut Value, id: i64) -> &mut Value {
        template["definitions"]["subgraphs"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["id"] == id)
            .unwrap()
    }

    #[test]
    fn recorded_moss_flattens_named_overrides_and_prunes_prompt_upsampler() {
        let mapped = adapt(&template("image_moss_image_t2i.json"), "MOSS-Image", &server()).unwrap();
        assert_eq!(mapped.model().as_deref(), Some("moss_image_sft_int8_convrot.safetensors"));
        assert_eq!(mapped.graph["t_18_7"]["inputs"]["steps"], 25);
        assert_eq!(mapped.graph["t_18_7"]["inputs"]["sampler_name"], "er_sde");
        assert_eq!(mapped.graph["t_18_7"]["inputs"]["cfg"], 2.5);
        assert_eq!(
            mapped.graph["t_18_5"]["inputs"]["text"],
            "nsfw, worst quality, bad anatomy, watermark, jpeg artifact"
        );
        assert!(mapped.graph.values().all(|node| !matches!(
            node["class_type"].as_str(),
            Some("TextGenerate" | "PreviewAny" | "ResolutionSelector" | "ComfySwitchNode")
        )));
        let filled = mapped.fill("A quiet mountain lake", 1536, 1024, 17).unwrap();
        assert_eq!(filled["t_18_4"]["inputs"]["text"], "A quiet mountain lake");
        assert_eq!(filled["t_18_6"]["inputs"]["width"], 1536);
        assert_eq!(filled["t_18_6"]["inputs"]["height"], 1024);
        assert_eq!(filled["t_18_7"]["inputs"]["seed"], 17);
        assert!(mapped.missing(&server()).is_empty());
    }

    #[test]
    fn recorded_z_image_positional_widgets_skip_seed_control_without_shifting_sampler() {
        let mapped = adapt(&template("image_z_image_turbo.json"), "Z-Image Turbo", &server()).unwrap();
        let sampler = &mapped.graph["t_57_3"]["inputs"];
        assert_eq!(sampler["steps"], 8);
        assert_eq!(sampler["cfg"], 1);
        assert_eq!(sampler["sampler_name"], "res_multistep");
        assert_eq!(sampler["scheduler"], "simple");
        assert_eq!(sampler["denoise"], 1);
        assert_eq!(mapped.graph["t_57_30"]["inputs"]["type"], "lumina2");
        assert_eq!(mapped.fill("Clouds", 1280, 720, 99).unwrap()["t_57_27"]["inputs"]["text"], "Clouds");
    }

    #[test]
    fn recorded_qwen_turbo_uses_manual_sigmas_and_stock_save_image() {
        let mut server = server();
        // This template's Turbo checkpoint was not installed on the recording machine. Simulate only
        // its installation, leaving the real node schemas and every other model choice unchanged.
        server["UNETLoader"]["input"]["required"]["unet_name"][0]
            .as_array_mut()
            .unwrap()
            .push(json!("qwen_image_2.1_turbo_int8_convrot.safetensors"));
        let mapped = adapt(&template("image_qwen_image_2_1_turbo_t2i.json"), "Qwen-Image 2.1 Turbo", &server).unwrap();
        assert_eq!(
            mapped.graph["t_459_484"]["inputs"]["sigmas"],
            "1.0, 0.978453, 0.95418, 0.926626, 0.89508, 0.845148, 0.704534, 0.414568, 0"
        );
        assert_eq!(mapped.graph["autopaper_save"]["class_type"], "SaveImage");
        assert_eq!(super::super::sampler_steps(&mapped.graph), 8);
        assert_eq!(mapped.graph["t_459_490"]["inputs"]["device"], "auto");
        assert_eq!(mapped.map.placeholders.seed.node, "t_459_488");
        assert_eq!(mapped.fill("Night sky", 1024, 768, 42).unwrap()["t_459_452"]["inputs"]["prompt"], "Night sky");
        assert!(!mapped.graph.contains_key("t_459_473"), "unused prompt-enhancement encoder must not be required");
    }

    #[test]
    fn unavailable_models_and_ambiguous_basename_paths_are_rejected() {
        assert!(adapt(&template("image_qwen_image_2_1_turbo_t2i.json"), "Qwen Turbo", &server()).is_err());
        let mut server = server();
        server["UNETLoader"]["input"]["required"]["unet_name"][0] =
            json!(["models/moss_image_sft_int8_convrot.safetensors"]);
        let mapped = adapt(&template("image_moss_image_t2i.json"), "MOSS", &server).unwrap();
        assert_eq!(mapped.model().as_deref(), Some("models/moss_image_sft_int8_convrot.safetensors"));
        server["UNETLoader"]["input"]["required"]["unet_name"][0] =
            json!(["a/moss_image_sft_int8_convrot.safetensors", "b/moss_image_sft_int8_convrot.safetensors"]);
        assert!(adapt(&template("image_moss_image_t2i.json"), "MOSS", &server).is_err());
    }

    #[test]
    fn unsafe_or_ambiguous_template_paths_are_rejected() {
        let original = template("image_moss_image_t2i.json");
        let mut media = original.clone();
        child_node(&mut media, 1)["type"] = json!("LoadImage");
        assert!(adapt(&media, "Media input", &server()).is_err());

        let mut api_server = server();
        api_server["UNETLoader"]["api_node"] = json!(true);
        assert!(adapt(&original, "API node", &api_server).is_err());

        let mut ambiguous = original.clone();
        let mut save = ambiguous["nodes"][0].clone();
        save["id"] = json!(999);
        ambiguous["nodes"].as_array_mut().unwrap().push(save);
        assert!(adapt(&ambiguous, "Two outputs", &server()).is_err());

        let mut muted = original.clone();
        child_node(&mut muted, 7)["mode"] = json!(4);
        assert!(adapt(&muted, "Bypass", &server()).is_err());

        let mut dynamic = original;
        dynamic["definitions"]["subgraphs"][0]["inputs"][1]["label"] = json!("unrecognized_switch");
        assert!(adapt(&dynamic, "Unknown prompt enhancer", &server()).is_err());
    }

    #[test]
    fn positional_mapping_requires_declared_order_and_correct_widget_count() {
        let mut no_order = server();
        no_order["KSampler"].as_object_mut().unwrap().remove("input_order");
        assert!(adapt(&template("image_z_image_turbo.json"), "Z-Image", &no_order).is_err());
        let mut extra = template("image_z_image_turbo.json");
        child_node(&mut extra, 3)["widgets_values"].as_array_mut().unwrap().push(json!(99));
        assert!(adapt(&extra, "Z-Image", &server()).is_err());
    }

    #[test]
    fn declared_dimensions_are_preserved_until_the_wallpaper_request_fills_them() {
        let mut template = template("image_moss_image_t2i.json");
        template["nodes"][1]["widgets_values_named"]["width"] = json!(1536);
        template["nodes"][1]["widgets_values_named"]["height"] = json!(768);
        let mapped = adapt(&template, "MOSS", &server()).unwrap();
        assert_eq!(mapped.graph["t_18_6"]["inputs"]["width"], 1536);
        assert_eq!(mapped.graph["t_18_6"]["inputs"]["height"], 768);
    }

    #[test]
    fn duplicate_ids_bad_links_and_recursive_subgraphs_are_rejected() {
        let mut duplicate = template("image_moss_image_t2i.json");
        let node = duplicate["nodes"][1].clone();
        duplicate["nodes"].as_array_mut().unwrap().push(node);
        assert!(adapt(&duplicate, "Duplicate", &server()).is_err());

        let mut broken = template("image_moss_image_t2i.json");
        broken["links"][0][3] = json!(12345);
        assert!(adapt(&broken, "Broken link", &server()).is_err());

        let mut recursive = template("image_moss_image_t2i.json");
        let definition = recursive["definitions"]["subgraphs"][0]["id"].clone();
        child_node(&mut recursive, 8)["type"] = definition;
        assert!(adapt(&recursive, "Recursive", &server()).is_err());
    }
}
