// Console check of the C# bindings: Engine on a temporary folder, Demo providers (the defaults), one keyword,
// one wallpaper. Exit code 0 when every step worked.
using System.Collections.Concurrent;
using System.Diagnostics;
using AutoPaper.Core;

var dataDir = Path.Combine(Path.GetTempPath(), "autopaper-smoke-" + Guid.NewGuid().ToString("N")[..8]);
var modelDir = args.Length > 0 ? args[0] : Path.Combine(AppContext.BaseDirectory, "no-model");
Console.WriteLine($"data dir:  {dataDir}");
Console.WriteLine($"model dir: {modelDir}");
var secrets = new DictionarySecrets();
var clock = Stopwatch.StartNew();
try
{
    using var engine = Engine.Open(new EngineConfig(dataDir, modelDir, "en-US", "Windows/smoke AutoPaper/0.1.0"), secrets);
    Console.WriteLine($"opened in {clock.ElapsedMilliseconds} ms");
    var memory = engine.MemoryStatus();
    Console.WriteLine($"memory: {memory.EmbeddingModel} (reduced: {memory.Reduced})");

    var settings = engine.Settings();
    Console.WriteLine($"providers: {settings.TextProvider.Kind} / {settings.ImageProvider.Kind}");
    engine.SetDisplayHint(2560, 1440);

    var keyword = engine.AddKeyword("rain", KeywordWeight.Must);
    engine.AddKeyword("lighthouse", KeywordWeight.Maybe);
    Console.WriteLine($"keywords: {string.Join(", ", engine.Keywords().Select(k => $"{k.Text} ({k.Weight})"))}");

    // A secret round trip through the foreign trait (the Demo provider never asks, so ask directly).
    var account = AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.OpenAiCompatible, "", "http://127.0.0.1:1234"));
    Console.WriteLine($"OpenAI-compatible account: {account}");

    var observer = new PrintingObserver();
    clock.Restart();
    var generation = await engine.Generate(Trigger.Manual, observer);
    Console.WriteLine($"generated in {clock.ElapsedMilliseconds} ms; stages: {string.Join(" > ", observer.Stages)} (on threads {string.Join(",", observer.Threads.Distinct())}; main thread {Environment.CurrentManagedThreadId})");
    Console.WriteLine($"title: {generation.Concept.Title}");
    Console.WriteLine($"image: {generation.ImagePath} ({generation.Width}x{generation.Height})");
    Console.WriteLine($"describe: {engine.Describe(generation.Id)}");

    var render = engine.RenderForDisplay(generation.Id, new DisplayTarget("smoke", 1920, 1080));
    Console.WriteLine($"render: {render} ({new FileInfo(render).Length:N0} bytes)");
    engine.MarkShown(generation.Id);
    Console.WriteLine($"current: {engine.Current()?.Concept.Title}");
    Console.WriteLine($"next due: {engine.NextDue()} (now {DateTimeOffset.UtcNow.ToUnixTimeSeconds()})");

    // Errors cross as typed exceptions.
    try
    {
        engine.Generation("no-such-id");
        Console.WriteLine("ERROR: expected NotFound");
        return 1;
    }
    catch (AutoPaperException.NotFound)
    {
        Console.WriteLine("typed error: NotFound as expected");
    }

    // Round 4: moods, progress detail, estimates and the early start, across the bindings.
    var detail = new DetailObserver();
    engine.SetProgressDetailObserver(detail);
    var mood = engine.CreateMood("Smoke test", engine.ActiveMood().Id);
    engine.SetActiveMood(mood.Id);
    engine.AddMoodKeyword(mood.Id, "harbour", KeywordWeight.Maybe);
    var second = await engine.Generate(Trigger.Manual, null);
    Console.WriteLine($"moods: {string.Join(", ", engine.Moods().Select(m => m.Active ? m.Name + " (current)" : m.Name))}; second wallpaper filed under {second.MoodName}");
    Console.WriteLine($"progress detail: {detail.Count} reports, last {detail.Last}");
    Console.WriteLine($"in this mood: {engine.HistoryByMood(HistoryFilter.All, mood.Id, 10, 0).Length}; estimate (images): {engine.Estimate(settings.ImageProvider, ProviderJob.Images, 0, 0)} s; next start {engine.NextStart()} <= next due {engine.NextDue()}");
    engine.SetProgressDetailObserver(null);
    if (detail.Count == 0 || second.MoodId != mood.Id)
    {
        Console.WriteLine("ERROR: progress detail or moods didn't cross the bindings");
        return 1;
    }

    var models = await engine.ListModels(settings.TextProvider, ProviderJob.Concepts);
    Console.WriteLine($"demo models: {string.Join(", ", models.Select(m => m.DisplayName))}");
    Console.WriteLine($"secret store calls: {secrets.Calls}");
    Console.WriteLine("SMOKE OK");
    return 0;
}
catch (Exception error)
{
    Console.WriteLine($"SMOKE FAILED: {error}");
    return 1;
}
finally
{
    try { Directory.Delete(dataDir, recursive: true); } catch (IOException) { }
}

sealed class DictionarySecrets : SecretStore
{
    private readonly ConcurrentDictionary<string, string> values = new();
    public int Calls;
    public string? Get(string account) { Interlocked.Increment(ref Calls); return values.TryGetValue(account, out var v) ? v : null; }
    public void Set(string account, string value) { Interlocked.Increment(ref Calls); values[account] = value; }
    public void Delete(string account) { Interlocked.Increment(ref Calls); values.TryRemove(account, out _); }
}

sealed class PrintingObserver : ProgressObserver
{
    public readonly ConcurrentQueue<ProgressStage> Stages = new();
    public readonly ConcurrentQueue<int> Threads = new();
    public void OnProgress(ProgressStage stage)
    {
        Stages.Enqueue(stage);
        Threads.Enqueue(Environment.CurrentManagedThreadId);
    }
}

sealed class DetailObserver : ProgressDetailObserver
{
    private int count;
    public int Count => count;
    public ProgressDetail? Last { get; private set; }
    public void OnProgressDetail(ProgressDetail detail)
    {
        Interlocked.Increment(ref count);
        Last = detail;
    }
}
