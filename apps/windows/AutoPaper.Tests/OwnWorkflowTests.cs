using AutoPaper.Core;
using AutoPaper.Models;

namespace AutoPaper.Tests;

/// <summary>A person's own ComfyUI workflow, read as the core reads it (core/src/providers/comfyui.rs: fill_placeholders,
/// api_graph, loader): the model its first loader loads (Settings' read-only Model line), and a file the core would
/// refuse is refused when it's chosen.</summary>
[TestClass]
public sealed class OwnWorkflowTests
{
    [TestMethod]
    public void TheModelItsFirstLoaderLoads()
    {
        const string checkpoint = """{"3": {"class_type": "KSampler", "inputs": {"seed": {{seed}}}}, "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "sd_xl_base_1.0.safetensors"}}, "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "masterpiece, {{prompt}}"}}, "5": {"class_type": "EmptyLatentImage", "inputs": {"width": "{{width}}", "height": {{height}}}}}""";
        Assert.AreEqual(new OwnWorkflow.Reading("sd_xl_base_1.0.safetensors", null), OwnWorkflow.Read(checkpoint));

        // A whole /prompt body works too; ckpt_name is looked for before unet_name, and node ids in number order.
        const string unet = """{"prompt": {"10": {"class_type": "UNETLoader", "inputs": {"unet_name": "qwen_image_2.1_int8_convrot.safetensors"}}, "4": {"class_type": "TextEncodeQwenImage21", "inputs": {"prompt": "{{prompt}}"}}}, "client_id": "x"}""";
        Assert.AreEqual("qwen_image_2.1_int8_convrot.safetensors", OwnWorkflow.Read(unet).Model);
        const string two = """{"9": {"class_type": "UNETLoader", "inputs": {"unet_name": "b.safetensors"}}, "10": {"class_type": "UNETLoader", "inputs": {"unet_name": "c.safetensors"}}, "2": {"class_type": "UNETLoader", "inputs": {"unet_name": "a.safetensors"}}, "1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}""";
        Assert.AreEqual("a.safetensors", OwnWorkflow.Read(two).Model);

        const string none = """{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}""";
        Assert.AreEqual(new OwnWorkflow.Reading(null, null), OwnWorkflow.Read(none));
    }

    [TestMethod]
    public void WhatTheCoreRefusesIsRefusedWhenChosen()
    {
        Assert.AreEqual(InvalidInputReason.WorkflowNeedsPrompt,
            OwnWorkflow.Read("""{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "fixed"}}}""").Problem);
        Assert.AreEqual(InvalidInputReason.WorkflowNotApiFormat,
            OwnWorkflow.Read("""{"last_node_id": 9, "nodes": [{"id": 1, "widgets_values": ["{{prompt}}"]}], "links": []}""").Problem);
        Assert.AreEqual(InvalidInputReason.WorkflowNotApiFormat, OwnWorkflow.Read("""{"text": "{{prompt}}"}""").Problem);
        Assert.AreEqual(InvalidInputReason.WorkflowNotApiFormat, OwnWorkflow.Read("""["{{prompt}}"]""").Problem);
        Assert.AreEqual(InvalidInputReason.WorkflowInvalid,
            OwnWorkflow.Read("""{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": {{prompt}}}}}""").Problem);
        Assert.AreEqual(InvalidInputReason.WorkflowInvalid, OwnWorkflow.Read("{{prompt}} not json").Problem);
    }

    [TestMethod]
    public void ModelFilesArePlainNames()
    {
        Assert.AreEqual("krea2_turbo_fp8_scaled", OwnWorkflow.PlainName("krea2_turbo_fp8_scaled.safetensors"));
        Assert.AreEqual("flux/dev", OwnWorkflow.PlainName("flux/dev.gguf"));
        Assert.AreEqual("custom", OwnWorkflow.PlainName("custom"));
    }

    /// <summary>AutoPaper's own workflows are read from the files the core is built with: each names the model its
    /// loader loads, and the core's default ComfyUI model is one of them (WordingTests: the picker's words).</summary>
    [TestMethod]
    public void BundledWorkflowsNameTheirModels()
    {
        Assert.HasCount(3, BundledWorkflows.All);
        var model = AutopaperCoreMethods.DefaultModel(ProviderKind.ComfyUi, ProviderJob.Images);
        Assert.AreEqual("Z-Image Turbo", BundledWorkflows.NameOf(model));
        Assert.AreEqual("Krea 2 Turbo", BundledWorkflows.NameOf("krea2_turbo_fp8_scaled.safetensors"));
        Assert.IsNull(BundledWorkflows.NameOf("not-one-of-ours.safetensors"));
    }
}
