import AutopaperCore
import Foundation

// ComfyUI's two kinds of workflow, as the Providers pane offers them: AutoPaper's own (one per model the core has a
// workflow for, listed by `list_models` only when the server has its files) or the person's own file. Problems with
// either are something to fix, not to wait out, so they're said once as a link to the fix (spec 6a).

/// The painter ran but couldn't paint, or the person's own ComfyUI workflow can't be used, and trying again won't
/// change that: one line with one link, "Check ComfyUI in Settings", which opens Settings on Providers. Worded from
/// the error's typed fields (`PaintingFailed`'s provider and model in words, an `InvalidInput` reason), never its
/// `detail`.
struct PaintingProblem: Equatable {
    enum Kind: Equatable {
        /// `PaintingFailed`: the provider rejected the workflow, a node failed, nothing was saved, or AutoPaper has
        /// no workflow for the chosen model.
        case couldntPaint
        /// The person's own ComfyUI workflow can't be used (`InvalidInput` with a workflow reason).
        case workflow(InvalidInputReason)
    }

    let kind: Kind
    let provider: ProviderKind
    /// The model in words ("Qwen-Image 2.1"), as the engine names it; empty when it isn't known.
    let model: String

    /// The problem behind `error` from making a wallpaper, when it's one of these; nil otherwise. `painting` is the
    /// painting provider in use and `ownWorkflow` whether ComfyUI runs the person's own file (only then is a workflow
    /// reason the person's to fix).
    init?(_ error: any Error, painting: ProviderSelection?, ownWorkflow: Bool) {
        switch error as? AutoPaperError {
        case .PaintingFailed(let provider, let model, _):
            kind = .couldntPaint
            self.provider = provider
            self.model = model
        case .InvalidInput(let reason, _):
            switch reason {
            case .workflowNeedsPrompt, .workflowNotApiFormat, .workflowInvalid:
                guard let painting, painting.kind == .comfyUi, ownWorkflow else { return nil }
                kind = .workflow(reason)
                provider = .comfyUi
                model = ""
            default:
                return nil
            }
        default:
            return nil
        }
    }

    /// "ComfyUI couldn't paint with Qwen-Image 2.1." (or without the model when it isn't known).
    static func sentence(provider: ProviderKind, model: String) -> String {
        let name = model.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? "\(provider.subject) couldn't paint." : "\(provider.subject) couldn't paint with \(name)."
    }

    /// The line before the link.
    var sentence: String {
        switch kind {
        case .couldntPaint:
            Self.sentence(provider: provider, model: model)
        case .workflow(.workflowNeedsPrompt):
            "Your ComfyUI workflow has no {{prompt}} placeholder for the scene."
        case .workflow(.workflowNotApiFormat):
            "Your ComfyUI workflow is in ComfyUI's editor format, not API format."
        case .workflow:
            "Your ComfyUI workflow isn't valid once AutoPaper fills in its placeholders."
        }
    }

    /// The link: the action itself, naming where it goes ("Check ComfyUI in Settings").
    var linkTitle: String {
        "Check \(provider.name) in Settings"
    }
}

/// AutoPaper's own ComfyUI workflows, by the model file each one loads, named as the core names them (the `name` in
/// core/resources/comfyui/*.map.json; a unit test checks the two agree). `list_models` gives these names when the
/// server answers; this is for when it can't be asked (not running, no key yet), so the Model menu still says
/// "Z-Image Turbo (default)" rather than a file name.
enum BundledWorkflows {
    static let names: [String: String] = [
        "z_image_turbo_bf16.safetensors": "Z-Image Turbo",
        "krea2_turbo_fp8_scaled.safetensors": "Krea 2 Turbo",
        "qwen_image_2.1_int8_convrot.safetensors": "Qwen-Image 2.1",
    ]

    /// A ComfyUI model in words: the server's name for it, else AutoPaper's own workflow's, else its file's name.
    static func name(of model: String, listed: [ModelInfo] = []) -> String {
        ModelMenu.names(listed)[model] ?? names[model] ?? OwnWorkflow.plainName(model)
    }
}

/// A person's own ComfyUI workflow file, read the way the core reads it when it fills one in (`fill_placeholders`,
/// `api_graph` and `loader` in core/src/providers/comfyui.rs), so a file that can't work is refused when it's
/// chosen rather than at the next wallpaper, and the pane can say which model it loads.
enum OwnWorkflow {
    enum Reading: Equatable {
        /// Usable; the model file its first loader loads (`ckpt_name`, else `unet_name`), if any.
        case model(String?)
        /// Can't be used, and why (`workflowNeedsPrompt`, `workflowNotApiFormat`, `workflowInvalid`).
        case problem(InvalidInputReason)
    }

    static func read(_ text: String) -> Reading {
        guard text.contains("{{prompt}}") else { return .problem(.workflowNeedsPrompt) }
        var filled = text
        for name in ["width", "height", "seed"] {
            filled = filled.replacingOccurrences(of: "\"{{\(name)}}\"", with: "1").replacingOccurrences(of: "{{\(name)}}", with: "1")
        }
        filled = filled.replacingOccurrences(of: "{{prompt}}", with: "")
        guard let data = filled.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) else {
            return .problem(.workflowInvalid)
        }
        guard var root = value as? [String: Any], !(root["nodes"] is [Any]) else { return .problem(.workflowNotApiFormat) }
        if let inner = root["prompt"] as? [String: Any], !root.values.allSatisfy(isNode) {
            root = inner
        }
        guard !root.isEmpty, root.values.allSatisfy(isNode) else { return .problem(.workflowNotApiFormat) }
        // Node ids in number order (then text), as the core looks for the loader.
        let ids = root.keys.sorted { (Int($0) ?? .max, $0) < (Int($1) ?? .max, $1) }
        for input in ["ckpt_name", "unet_name"] {
            for id in ids {
                if let inputs = (root[id] as? [String: Any])?["inputs"] as? [String: Any], let model = inputs[input] as? String {
                    return .model(model)
                }
            }
        }
        return .model(nil)
    }

    private static func isNode(_ value: Any) -> Bool {
        guard let node = value as? [String: Any], node["class_type"] is String else { return false }
        return node["inputs"] == nil || node["inputs"] is [String: Any]
    }

    /// "krea2_turbo_fp8_scaled.safetensors" → "krea2_turbo_fp8_scaled" (the core's `display_name`); folders are kept.
    static func plainName(_ file: String) -> String {
        for suffix in [".safetensors", ".ckpt", ".gguf", ".pt", ".pth", ".bin"] where file.hasSuffix(suffix) {
            return String(file.dropLast(suffix.count))
        }
        return file
    }

    /// The read-only Model line for a person's own workflow.
    static func modelLine(_ reading: Reading?) -> String {
        if case .model(let file?) = reading { return "\(plainName(file)) (from your workflow)" }
        return "Set in your workflow"
    }
}

/// The two-way choice in Providers: AutoPaper's workflow or the person's own file. The file is remembered (its text
/// and name, in this app's preferences) after switching back to AutoPaper's, so it's one choice away; the engine
/// only holds the workflow in use (`EngineSettings.comfyuiWorkflow`).
enum WorkflowChoice: Hashable {
    case autoPaper
    case own
    /// "Your Own…" / "Choose Another…": opens the file panel; the choice itself doesn't change until a file is used.
    case chooseFile

    enum Keys {
        static let text = "comfyuiOwnWorkflowText"
        static let name = "comfyuiOwnWorkflowName"
    }

    static let autoPaperTitle = "AutoPaper's"

    /// The own workflow's menu item: its file name, or (chosen before names were kept) a description.
    static func ownTitle(fileName: String?) -> String {
        guard let fileName, !fileName.isEmpty else { return "Your Own Workflow" }
        return fileName
    }

    /// The file panel's menu item: "Your Own…" until a file has been chosen, then "Choose Another…".
    static func chooseTitle(hasFile: Bool) -> String {
        hasFile ? "Choose Another…" : "Your Own…"
    }
}
