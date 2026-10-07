using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Navigation;
using Windows.System;
using CoreSettings = AutoPaper.Core.Settings;

namespace AutoPaper.Views.Panes;

/// <summary>
/// Settings › Providers: who writes the ideas and who paints them. Per job: the provider, its server address
/// (local and OpenAI-compatible kinds), the model (from list_models, with Refresh, and how long it takes on this PC
/// from estimate), painting quality, ComfyUI's workflow (AutoPaper's, or the person's own file with its model on a
/// read-only line), Test with the result in words, and where to get Ollama or ComfyUI for Windows (each project's
/// own installer; Homebrew and Brew Browser don't run on Windows).
/// A problem is said once per job, in one place, whether listing models or Test found it (docs/app-spec.md 6a); a
/// missing or refused key is a link to its field ("Add your OpenAI key"), once on the page, and Test and Refresh are
/// disabled until it's fixed. Saving the key lists the models again.
/// </summary>
public sealed partial class ProvidersPage : Page
{
    private const string OllamaDownload = "https://ollama.com/download/windows";
    private const string ComfyDownload = "https://www.comfy.org/download";
    /// <summary>A ComfyUI workflow file larger than this isn't an API-format workflow.</summary>
    private const long MaxWorkflowBytes = 1024 * 1024;

    private readonly Group text;
    private readonly Group image;
    /// <summary>Keys a provider refused (InvalidKey) since they were last saved.</summary>
    private readonly HashSet<string> refusedKeys = [];
    private bool loading = true;
    /// <summary>The page has shown its first state: a problem that appears from now on is said, not only shown.</summary>
    private bool shown;
    /// <summary>Why the workflow file just chosen (or the person's workflow in use) can't be used: shown on the
    /// Workflow card in place of its description until the next choice.</summary>
    private string? workflowProblem;

    public ProvidersPage()
    {
        InitializeComponent();
        text = new Group(ProviderJob.Concepts, Text.Writers,
            TextKindCard, TextKind, TextAddressCard, TextAddress, TextModelCard, TextModel, TextRefresh, TextTestCard, TextTest, TextUnavailable, TextKeyLink, TextInstallCard);
        image = new Group(ProviderJob.Images, Text.Painters,
            ImageKindCard, ImageKind, ImageAddressCard, ImageAddress, ImageModelCard, ImageModel, ImageRefresh, ImageTestCard, ImageTest, ImageUnavailable, ImageKeyLink, ImageInstallCard);
        foreach (var group in Groups)
        {
            foreach (var kind in group.Kinds)
            {
                group.Kind.Items.Add(new ComboBoxItem { Content = Text.ProviderOption(kind), Tag = kind });
            }
            group.InstallCard.ActionIconToolTip = Loc.Get("Install_OpenInBrowser");
            // Both groups share these cards' headers; their spoken names say which job they're for (a card without a
            // name is named after its content by UI Automation, such as the address field's placeholder).
            var job = group.Job == ProviderJob.Concepts ? "Writer" : "Painter";
            AutomationProperties.SetName(group.AddressCard, Loc.Get($"Card_Address{job}"));
            AutomationProperties.SetName(group.Address, Loc.Get($"Card_Address{job}"));
            AutomationProperties.SetName(group.ModelCard, Loc.Get($"Card_Model{job}"));
            AutomationProperties.SetName(group.TestCard, Loc.Get($"Card_Test{job}"));
        }
        Loaded += (_, _) => Model.KeyChanged += OnKeyChanged;
        Unloaded += (_, _) => Model.KeyChanged -= OnKeyChanged;
    }

    private static AppModel Model => App.Model;

    private IEnumerable<Group> Groups => [text, image];

    private static bool UsingOwnWorkflow => !string.IsNullOrWhiteSpace(Model.Settings.ComfyuiWorkflow);

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        PaneHeader.Attach(Crumbs, "Providers", Frame);
        if (!Model.IsReady)
        {
            return;
        }
        ShowSettings();
        loading = false;
        await Task.WhenAll(LoadModelsAsync(text), LoadModelsAsync(image));
        shown = true;
    }

    private Group GroupOf(object? control) => text.Owns(control) ? text : image;

    /// <summary>Shows the engine's settings in the controls (and which cards apply to each provider).</summary>
    private void ShowSettings()
    {
        var was = loading;
        loading = true;
        var settings = Model.Settings;
        foreach (var group in Groups)
        {
            var selection = group.Selection(settings);
            group.Kind.SelectedIndex = Array.IndexOf(group.Kinds, selection.Kind);
            var hasAddress = selection.Kind is ProviderKind.Ollama or ProviderKind.ComfyUi or ProviderKind.OpenAiCompatible;
            group.AddressCard.Visibility = Visible(hasAddress);
            group.Address.Text = selection.BaseUrl ?? "";
            // The address used when the field is empty (the core's default_base_url); OpenAI-compatible has none.
            group.Address.PlaceholderText = AutopaperCoreMethods.DefaultBaseUrl(selection.Kind) ?? Loc.Get("Address_CompatiblePlaceholder");
            group.AddressCard.Description = Loc.Get(selection.Kind == ProviderKind.OpenAiCompatible ? "Address_CompatibleDescription" : "Address_LocalDescription");
            group.ModelCard.Visibility = Visible(selection.Kind != ProviderKind.Demo);
            group.TestCard.Description = Loc.Get("Test_Description");
            group.InstallCard.Visibility = Visible(selection.Kind is ProviderKind.Ollama or ProviderKind.ComfyUi);
            group.InstallCard.Header = Loc.Get(selection.Kind == ProviderKind.Ollama ? "Install_OllamaHeader" : "Install_ComfyHeader");
            group.InstallCard.Description = Loc.Get(selection.Kind == ProviderKind.Ollama ? "Install_OllamaDescription" : "Install_ComfyDescription");
        }
        var painter = settings.ImageProvider.Kind;
        ImageQuality.SelectedIndex = settings.ImageQuality == Core.ImageQuality.High ? 1 : 0;
        ImageQualityCard.Visibility = Visible(painter is ProviderKind.OpenAi or ProviderKind.Google or ProviderKind.OpenAiCompatible);
        ShowWorkflow();
        GeminiNote.IsOpen = settings.TextProvider.Kind == ProviderKind.Google || painter == ProviderKind.Google;
        ShowProblems();
        loading = was;
    }

    private static Visibility Visible(bool value) => value ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>Saves one change to a job's settings (<paramref name="change"/> runs on the thread pool: it uses
    /// values read before). One that didn't save is that job's one problem, in its bar next to its cards (the bar says
    /// it as it opens); the caller puts its control back.</summary>
    private async Task<bool> SaveAsync(Group group, Func<CoreSettings, CoreSettings> change)
    {
        try
        {
            await Model.UpdateSettingsAsync(change);
            return true;
        }
        catch (Exception error)
        {
            if (error is not AutoPaperException)
            {
                Log.Error("Saving provider settings", error);
            }
            group.Problem = Text.Problem(error, Model.Settings, inSettings: true).Spoken;
            ShowProblems();
            return false;
        }
    }

    // ── Provider ────────────────────────────────────────────────────────────────────────────────

    private async void OnKindChanged(object sender, SelectionChangedEventArgs e)
    {
        var group = GroupOf(sender);
        if (loading || group.Kind.SelectedItem is not ComboBoxItem { Tag: ProviderKind kind } || kind == group.Selection(Model.Settings).Kind)
        {
            return;
        }
        // A new provider starts on its default model and address, with nothing said about the old one.
        if (await SaveAsync(group, settings => group.With(settings, new ProviderSelection(kind, "", null))))
        {
            group.Problem = null;
            if (group == image)
            {
                workflowProblem = null;
            }
            ShowSettings();
            await LoadModelsAsync(group);
        }
        else
        {
            ShowSettings();
        }
    }

    // ── Address ─────────────────────────────────────────────────────────────────────────────────

    private async void OnAddressKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            e.Handled = true;
            await SaveAddressAsync(GroupOf(sender));
        }
    }

    private async void OnAddressLostFocus(object sender, RoutedEventArgs e) => await SaveAddressAsync(GroupOf(sender));

    private async Task SaveAddressAsync(Group group)
    {
        if (loading)
        {
            return;
        }
        var typed = group.Address.Text.Trim();
        var current = group.Selection(Model.Settings);
        if (typed == (current.BaseUrl ?? ""))
        {
            return;
        }
        try
        {
            await Model.UpdateSettingsAsync(settings =>
                group.With(settings, current with { BaseUrl = typed.Length == 0 ? null : typed }));
            group.AddressCard.Description = Loc.Get(current.Kind == ProviderKind.OpenAiCompatible ? "Address_CompatibleDescription" : "Address_LocalDescription");
            await LoadModelsAsync(group);
        }
        catch (Exception error)
        {
            // Worded by its reason (missing, not allowed, not a complete address), under the field it's about.
            if (error is not AutoPaperException)
            {
                Log.Error("Saving a provider's address", error);
            }
            var problem = Text.Error(error, Model.Settings);
            group.AddressCard.Description = PaneHeader.ProblemText(problem);
            Model.AnnounceError(problem);
        }
    }

    // ── Keys: the one link per missing or refused key ───────────────────────────────────────────

    /// <summary>The key a job can't work without that's missing or was refused: its account, and whether it was
    /// refused (else missing). Null when the provider has its key, or needs none.</summary>
    private (string Account, ProviderKind Kind, bool Refused)? KeyBlock(Group group)
    {
        var selection = group.Selection(Model.Settings);
        if (AutopaperCoreMethods.SecretAccountFor(selection) is not { } account)
        {
            return null;
        }
        if (refusedKeys.Contains(account))
        {
            return (account, selection.Kind, true);
        }
        if (Text.NeedsKey(selection.Kind) && string.IsNullOrWhiteSpace(Model.Secrets.Get(account)))
        {
            return (account, selection.Kind, false);
        }
        return null;
    }

    /// <summary>
    /// Each job's one problem: a key link ("Add your OpenAI key"; shown once when both jobs need the same key), or
    /// what listing models or Test found last. Test and Refresh are disabled while a key blocks them; focus leaves a
    /// control before it's disabled. A bar that opens announces itself; a link-only one is said by the window.
    /// </summary>
    private void ShowProblems()
    {
        string? linked = null;
        foreach (var group in Groups)
        {
            var block = KeyBlock(group);
            string message;
            string? link = null;
            if (block is { } key)
            {
                if (key.Account == linked)
                {
                    message = "";
                }
                else
                {
                    message = "";
                    link = Text.KeyLink(key.Kind, key.Refused);
                    linked = key.Account;
                }
                group.KeyLink.Tag = key.Account;
            }
            else
            {
                message = group.Problem ?? "";
            }
            ShowBar(group, message, link);
            SetBlocked(group, block is not null);
        }
    }

    private static void SetBlocked(Group group, bool blocked)
    {
        foreach (var button in new[] { group.Test, group.Refresh })
        {
            if (blocked && button.FocusState != FocusState.Unfocused)
            {
                // Keyboard focus goes to the fix (or the model picker), not to the title bar.
                var showsLink = group.KeyLink.Visibility == Visibility.Visible && group.Unavailable.IsOpen;
                if (!(showsLink && group.KeyLink.Focus(button.FocusState)))
                {
                    group.ModelBox.Focus(button.FocusState);
                }
            }
            button.IsEnabled = !blocked;
        }
    }

    private void ShowBar(Group group, string message, string? link)
    {
        var open = message.Length > 0 || link is not null;
        var changed = group.Unavailable.Message != message || (group.KeyLink.Content as string ?? "") != (link ?? "") || group.Unavailable.IsOpen != open;
        group.KeyLink.Content = link ?? "";
        group.KeyLink.Visibility = Visible(link is not null);
        if (!changed)
        {
            return;
        }
        // Closed first, so a new problem announces itself even when the bar was open.
        group.Unavailable.IsOpen = false;
        group.Unavailable.Message = message;
        group.Unavailable.IsOpen = open;
        if (open && message.Length == 0 && link is not null && shown)
        {
            // The bar says only "Warning" when it opens without a sentence: the link's words are said here.
            Model.AnnounceError(link);
        }
    }

    private void OnKeyLink(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: string account })
        {
            Frame.Navigate(typeof(KeysPage), account, new SlideNavigationTransitionInfo { Effect = SlideNavigationTransitionEffect.FromRight });
        }
    }

    /// <summary>A key was saved (here, Settings › Keys, the first-run page): it's no longer known to be refused, and
    /// the jobs that use it list their models again.</summary>
    private async void OnKeyChanged(object? sender, string account)
    {
        refusedKeys.Remove(account);
        foreach (var group in Groups.Where(group => AutopaperCoreMethods.SecretAccountFor(group.Selection(Model.Settings)) == account).ToList())
        {
            group.Problem = null;
            await LoadModelsAsync(group);
        }
    }

    // ── Model ───────────────────────────────────────────────────────────────────────────────────

    private async void OnRefresh(object sender, RoutedEventArgs e) => await LoadModelsAsync(GroupOf(sender), announce: true);

    /// <summary>Fills the model picker from list_models: "Default (…)" first, then what the provider offers, and the
    /// chosen model even when the provider didn't list it. Refresh stays enabled while it lists (disabling the
    /// focused button would throw keyboard focus elsewhere); a request made meanwhile lists again afterwards.</summary>
    private async Task LoadModelsAsync(Group group, bool announce = false)
    {
        if (group.Listing)
        {
            group.ListAgain = true;
            group.AnnounceList |= announce;
            return;
        }
        group.Listing = true;
        try
        {
            do
            {
                group.ListAgain = false;
                await ListModelsOnceAsync(group, announce || group.AnnounceList);
                group.AnnounceList = false;
            }
            while (group.ListAgain);
        }
        finally
        {
            group.Listing = false;
        }
    }

    private async Task ListModelsOnceAsync(Group group, bool announce)
    {
        var selection = group.Selection(Model.Settings);
        group.KindCard.Description = group.KindDescription;
        if (selection.Kind == ProviderKind.Demo)
        {
            // No models to choose: how long Demo takes here goes on its card.
            group.Problem = null;
            var estimate = await EstimateLineAsync(group, null);
            group.KindCard.Description = string.Join(" ", new[] { group.KindDescription, estimate }.Where(part => part.Length > 0));
            ShowProblems();
            return;
        }
        if (selection.Kind == ProviderKind.OpenAiCompatible && string.IsNullOrWhiteSpace(selection.BaseUrl))
        {
            group.ModelCard.Description = Loc.Get("Models_NeedAddress");
            FillModels(group, []);
            return;
        }
        if (KeyBlock(group) is not null)
        {
            // Nothing to ask without the key: the link says what to do; the picker keeps its default.
            group.Models = [];
            FillModels(group, []);
            group.ModelCard.Description = await EstimateLineAsync(group, null);
            ShowProblems();
            return;
        }
        group.ModelCard.Description = Loc.Get("Models_Listing");
        string? listed = null;
        try
        {
            group.Models = [.. await Model.CallAsync(e => e.ListModels(selection, group.Job))];
            Model.RememberModelNames(group.Models);
            listed = group.Models.Count == 0 ? Loc.Get("Models_NoneListed") : Loc.Format("Models_Listed", group.Models.Count);
            group.Problem = null;
        }
        catch (Exception error)
        {
            group.Models = [];
            NoteProblem(group, selection, error);
        }
        FillModels(group, group.Models);
        group.ModelCard.Description = await EstimateLineAsync(group, listed);
        ShowProblems();
        if (announce && listed is not null)
        {
            Model.Announce(listed);
        }
    }

    /// <summary>A problem listing models or testing: a refused key becomes its link; anything else is said in the
    /// job's bar, worded for Settings (no "in Settings ›…"). Ollama or ComfyUI not running also points at the
    /// installer card below.</summary>
    private void NoteProblem(Group group, ProviderSelection selection, Exception error)
    {
        if (error is AutoPaperException.InvalidKey && AutopaperCoreMethods.SecretAccountFor(selection) is { } account)
        {
            refusedKeys.Add(account);
            group.Problem = null;
            return;
        }
        if (error is AutoPaperException.MissingKey)
        {
            group.Problem = null; // The key link says it (KeyBlock).
            return;
        }
        if (error is AutoPaperException.ProviderUnavailable { reason: ProviderUnavailableReason.NotRunning }
            && selection.Kind is ProviderKind.Ollama or ProviderKind.ComfyUi)
        {
            group.Problem = Loc.Format("Install_NotRunning", Text.Provider(selection.Kind), Text.Address(selection.Kind, Model.Settings));
            return;
        }
        if (error is AutoPaperException.InvalidInput
            {
                reason: InvalidInputReason.WorkflowNeedsPrompt or InvalidInputReason.WorkflowNotApiFormat or InvalidInputReason.WorkflowInvalid,
            } && group.Job == ProviderJob.Images)
        {
            // The person's own workflow can't be used: said on its card, where it's changed.
            ShowWorkflowProblem(Text.Error(error, Model.Settings));
            group.Problem = null;
            return;
        }
        // A test is the person checking now: no "AutoPaper will try again later".
        group.Problem = Text.Problem(error, Model.Settings, scheduled: false, inSettings: true).Spoken;
    }

    private void FillModels(Group group, List<ModelInfo> models)
    {
        var selection = group.Selection(Model.Settings);
        group.DefaultChoice = Text.DefaultModelChoice(selection.Kind, group.Job, models);
        var was = loading;
        loading = true;
        group.ModelBox.Items.Clear();
        group.ModelBox.Items.Add(group.DefaultChoice);
        foreach (var model in models)
        {
            group.ModelBox.Items.Add(model.DisplayName);
        }
        var chosen = selection.Model;
        if (chosen.Length == 0)
        {
            group.ModelBox.SelectedIndex = 0;
        }
        else
        {
            var index = models.FindIndex(model => model.Id == chosen);
            if (index < 0)
            {
                group.ModelBox.Items.Add(chosen);
                group.ModelBox.SelectedIndex = group.ModelBox.Items.Count - 1;
            }
            else
            {
                group.ModelBox.SelectedIndex = index + 1;
            }
        }
        loading = was;
    }

    /// <summary>The model card's line: how many models are listed, and how long one takes here (estimate: the writer's
    /// per idea; the painter's per wallpaper, with the idea's writing when that's known too). Nothing is made up: no
    /// timing recorded on this PC, no estimate.</summary>
    private async Task<string> EstimateLineAsync(Group group, string? listed)
    {
        var settings = Model.Settings;
        uint? seconds = null;
        try
        {
            seconds = await Model.Call(e =>
            {
                var own = e.Estimate(group.Selection(settings), group.Job, 0, 0);
                if (group.Job == ProviderJob.Images && own is { } painting && e.Estimate(settings.TextProvider, ProviderJob.Concepts, 0, 0) is { } writing)
                {
                    return painting + writing;
                }
                return own;
            });
        }
        catch (Exception error)
        {
            // An estimate is a nicety: a provider the core can't build here (no address yet) has none.
            if (error is not AutoPaperException)
            {
                Log.Error("Estimating", error);
            }
        }
        var estimate = Text.Estimate(group.Job, seconds);
        return string.Join(" ", new[] { listed ?? "", estimate }.Where(part => part.Length > 0));
    }

    private async void OnModelChanged(object sender, SelectionChangedEventArgs e)
    {
        var group = GroupOf(sender);
        if (loading || group.ModelBox.SelectedIndex < 0)
        {
            return;
        }
        var index = group.ModelBox.SelectedIndex;
        var id = index == 0 ? "" : index - 1 < group.Models.Count ? group.Models[index - 1].Id : group.ModelBox.SelectedItem as string ?? "";
        await SaveModelAsync(group, id);
    }

    /// <summary>A model typed in (one the provider didn't list).</summary>
    private async void OnModelTextSubmitted(ComboBox sender, ComboBoxTextSubmittedEventArgs args)
    {
        var group = GroupOf(sender);
        var typed = args.Text.Trim();
        var listed = group.Models.FirstOrDefault(model => model.DisplayName == typed || model.Id == typed);
        var id = typed == group.DefaultChoice || typed == Loc.Get("Models_Default") ? "" : listed?.Id ?? typed;
        args.Handled = true;
        await SaveModelAsync(group, id);
        await LoadModelsAsync(group);
    }

    private async Task SaveModelAsync(Group group, string id)
    {
        var current = group.Selection(Model.Settings);
        if (current.Model == id)
        {
            return;
        }
        if (await SaveAsync(group, settings => group.With(settings, current with { Model = id })))
        {
            // The estimate is per model.
            var listed = group.Models.Count == 0 ? null : Loc.Format("Models_Listed", group.Models.Count);
            group.ModelCard.Description = await EstimateLineAsync(group, listed);
        }
        else
        {
            FillModels(group, group.Models); // The picker shows the saved model again.
        }
    }

    // ── Painting quality and ComfyUI's workflow ─────────────────────────────────────────────────

    private async void OnQualityChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!loading)
        {
            var quality = ImageQuality.SelectedIndex == 1 ? Core.ImageQuality.High : Core.ImageQuality.Standard;
            if (!await SaveAsync(image, settings => settings with { ImageQuality = quality }))
            {
                ShowSettings();
            }
        }
    }

    /// <summary>The Workflow choice, its file card and the Model card's two forms: AutoPaper's (the menu) or the
    /// person's own (a read-only line naming the model its file loads).</summary>
    private void ShowWorkflow()
    {
        var comfy = Model.Settings.ImageProvider.Kind == ProviderKind.ComfyUi;
        var own = comfy && UsingOwnWorkflow;
        var was = loading;
        loading = true;
        ImageWorkflowCard.Visibility = Visible(comfy);
        WorkflowBox.SelectedIndex = own ? 1 : 0;
        // A file that was just refused (or the workflow in use can't be used) says why here until the next choice.
        ImageWorkflowCard.Description = workflowProblem is { } problem && comfy
            ? PaneHeader.ProblemText(problem)
            : Loc.Get(own ? "Workflow_OwnDescription" : "Workflow_AutoPaperDescription");
        WorkflowFileCard.Visibility = Visible(own);
        WorkflowFileCard.Description = Preferences.OwnWorkflowName is { Length: > 0 } name ? name : Loc.Get("Workflow_OwnUnnamed");
        ImageModelPicker.Visibility = Visible(!own);
        ImageModelLine.Visibility = Visible(own);
        if (own)
        {
            var reading = Models.OwnWorkflow.Read(Model.Settings.ComfyuiWorkflow!);
            ImageModelLine.Text = reading.Model is { } file
                ? Loc.Format("Workflow_ModelLine", Models.OwnWorkflow.PlainName(file))
                : Loc.Get("Workflow_ModelUnknown");
            AutomationProperties.SetName(ImageModelLine, Loc.Format("Workflow_ModelLineSpoken", ImageModelLine.Text));
        }
        loading = was;
    }

    /// <summary>AutoPaper's, or Your own… (the remembered file when there is one, else a file picker; cancelling
    /// it keeps AutoPaper's).</summary>
    private async void OnWorkflowChanged(object sender, SelectionChangedEventArgs e)
    {
        if (loading || WorkflowBox.SelectedIndex < 0)
        {
            return;
        }
        var wantOwn = WorkflowBox.SelectedIndex == 1;
        if (wantOwn == UsingOwnWorkflow)
        {
            return;
        }
        if (!wantOwn)
        {
            await UseAutoPaperWorkflowAsync();
            return;
        }
        var remembered = await RememberedWorkflowAsync();
        if (remembered is not null)
        {
            await UseOwnWorkflowAsync(remembered, Preferences.OwnWorkflowName);
        }
        else
        {
            await ChooseWorkflowFileAsync();
        }
        ShowWorkflow();
    }

    private async void OnWorkflowChoose(object sender, RoutedEventArgs e)
    {
        await ChooseWorkflowFileAsync();
        ShowWorkflow();
    }

    private static async Task<string?> RememberedWorkflowAsync()
    {
        try
        {
            return File.Exists(Preferences.OwnWorkflowFile) ? await File.ReadAllTextAsync(Preferences.OwnWorkflowFile) : null;
        }
        catch (Exception error)
        {
            Log.Error("Reading the remembered workflow", error);
            return null;
        }
    }

    /// <summary>The file picker (JSON). A file that can't work is refused now, with why, rather than at the next
    /// wallpaper (read as the core reads it: OwnWorkflow).</summary>
    private async Task ChooseWorkflowFileAsync()
    {
        var picker = new Microsoft.Windows.Storage.Pickers.FileOpenPicker(App.Window.AppWindow.Id);
        picker.FileTypeFilter.Add(".json");
        var picked = await picker.PickSingleFileAsync();
        if (picked is null)
        {
            return;
        }
        string workflow;
        try
        {
            if (new FileInfo(picked.Path).Length > MaxWorkflowBytes)
            {
                throw new InvalidDataException();
            }
            workflow = await File.ReadAllTextAsync(picked.Path);
        }
        catch (Exception error)
        {
            Log.Error("Reading a workflow file", error);
            ShowWorkflowProblem(Loc.Get("Workflow_NotJson"));
            return;
        }
        if (Models.OwnWorkflow.Read(workflow).Problem is { } reason)
        {
            ShowWorkflowProblem(Text.InvalidInput(reason));
            return;
        }
        await UseOwnWorkflowAsync(workflow, Path.GetFileName(picked.Path));
    }

    /// <summary>Uses the person's workflow: its text goes to the engine (the model it loads decides, so the model
    /// choice goes back to its default) and is remembered here with its name.</summary>
    private async Task UseOwnWorkflowAsync(string workflow, string name)
    {
        if (!await SaveAsync(image, settings => settings with
            {
                ComfyuiWorkflow = workflow,
                ImageProvider = settings.ImageProvider with { Model = "" },
            }))
        {
            return;
        }
        workflowProblem = null;
        Preferences.OwnWorkflowName = name;
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(Preferences.OwnWorkflowFile)!);
            await File.WriteAllTextAsync(Preferences.OwnWorkflowFile, workflow);
        }
        catch (Exception error)
        {
            Log.Error("Remembering the workflow", error); // It's in use; only the switch back is lost.
        }
        ShowSettings();
        Model.Announce(Loc.Format("Workflow_Using", name.Length > 0 ? name : Loc.Get("Workflow_OwnUnnamed")));
        image.Problem = null;
        await LoadModelsAsync(image);
    }

    private async Task UseAutoPaperWorkflowAsync()
    {
        if (await SaveAsync(image, settings => settings with { ComfyuiWorkflow = null, ImageProvider = settings.ImageProvider with { Model = "" } }))
        {
            workflowProblem = null;
            ShowSettings();
            Model.Announce(Loc.Get("Workflow_UsingAutoPaper"));
            image.Problem = null;
            await LoadModelsAsync(image);
        }
        else
        {
            ShowWorkflow();
        }
    }

    /// <summary>Why a workflow can't be used, on the Workflow card (kept there until the next choice: ShowWorkflow
    /// shows it in place of the card's description) and said once.</summary>
    private void ShowWorkflowProblem(string message)
    {
        workflowProblem = message;
        ShowWorkflow();
        Model.AnnounceError(message);
    }

    // ── Test, install, keys ─────────────────────────────────────────────────────────────────────

    /// <summary>Test, with the result in words: success on the Test card; a problem in the job's one bar (replacing
    /// what listing models said). The button stays enabled while it runs (disabling the focused button would throw
    /// keyboard focus to the next control); pressing it again meanwhile does nothing.</summary>
    private async void OnTest(object sender, RoutedEventArgs e)
    {
        var group = GroupOf(sender);
        if (group.Testing)
        {
            return;
        }
        group.Testing = true;
        var selection = group.Selection(Model.Settings);
        group.TestCard.Description = Loc.Get("Test_Running");
        Model.Announce(Loc.Get("Test_Running"));
        string? result = null;
        try
        {
            await Model.CallAsync(engine => engine.TestProvider(selection, group.Job));
            result = Loc.Format("Test_Ok", Text.Provider(selection.Kind));
            group.Problem = null;
        }
        catch (Exception error)
        {
            NoteProblem(group, selection, error);
        }
        finally
        {
            group.Testing = false;
        }
        group.TestCard.Description = result ?? Loc.Get("Test_Description");
        var wasOpen = group.Unavailable.IsOpen;
        var before = group.Unavailable.Message;
        ShowProblems();
        if (result is not null)
        {
            Model.Announce(result);
        }
        else if (wasOpen && group.Unavailable.IsOpen && before == group.Unavailable.Message && group.Unavailable.Message.Length > 0)
        {
            // The same problem as before: the bar didn't reopen, so it's said again here.
            Model.AnnounceError(group.Unavailable.Message);
        }
    }

    private async void OnInstall(object? sender, EventArgs e)
    {
        var group = GroupOf(sender);
        var kind = group.Selection(Model.Settings).Kind;
        await Launcher.LaunchUriAsync(new Uri(kind == ProviderKind.Ollama ? OllamaDownload : ComfyDownload));
    }

    private void OnOpenKeys(object? sender, EventArgs e) =>
        Frame.Navigate(typeof(KeysPage), null, new SlideNavigationTransitionInfo { Effect = SlideNavigationTransitionEffect.FromRight });

    /// <summary>One job's controls (writing ideas, or painting).</summary>
    private sealed class Group(
        ProviderJob job,
        ProviderKind[] kinds,
        SettingsCard kindCard,
        ComboBox kind,
        SettingsCard addressCard,
        TextBox address,
        SettingsCard modelCard,
        ComboBox modelBox,
        Button refresh,
        SettingsCard testCard,
        Button test,
        InfoBar unavailable,
        HyperlinkButton keyLink,
        SettingsCard installCard)
    {
        public ProviderJob Job { get; } = job;
        public ProviderKind[] Kinds { get; } = kinds;
        public SettingsCard KindCard { get; } = kindCard;
        /// <summary>The provider card's own description ("Paints the scene."), before any estimate.</summary>
        public string KindDescription { get; } = kindCard.Description as string ?? "";
        public ComboBox Kind { get; } = kind;
        public SettingsCard AddressCard { get; } = addressCard;
        public TextBox Address { get; } = address;
        public SettingsCard ModelCard { get; } = modelCard;
        public ComboBox ModelBox { get; } = modelBox;
        public Button Refresh { get; } = refresh;
        public SettingsCard TestCard { get; } = testCard;
        public Button Test { get; } = test;
        /// <summary>The job's one problem bar.</summary>
        public InfoBar Unavailable { get; } = unavailable;
        /// <summary>The bar's link to a key's field ("Add your OpenAI key").</summary>
        public HyperlinkButton KeyLink { get; } = keyLink;
        public SettingsCard InstallCard { get; } = installCard;
        public List<ModelInfo> Models { get; set; } = [];
        /// <summary>The model picker's blank choice, as shown ("Default (gpt-6-luna)").</summary>
        public string DefaultChoice { get; set; } = "";
        /// <summary>What listing models or Test found wrong last (not a key: that's the link), in words.</summary>
        public string? Problem { get; set; }
        public bool Testing { get; set; }
        public bool Listing { get; set; }
        public bool ListAgain { get; set; }
        public bool AnnounceList { get; set; }

        public bool Owns(object? control) => ReferenceEquals(control, Kind) || ReferenceEquals(control, Address)
            || ReferenceEquals(control, ModelBox) || ReferenceEquals(control, Refresh) || ReferenceEquals(control, Test)
            || ReferenceEquals(control, InstallCard) || ReferenceEquals(control, KeyLink);

        public ProviderSelection Selection(CoreSettings settings) =>
            Job == ProviderJob.Concepts ? settings.TextProvider : settings.ImageProvider;

        public CoreSettings With(CoreSettings settings, ProviderSelection selection) =>
            Job == ProviderJob.Concepts ? settings with { TextProvider = selection } : settings with { ImageProvider = selection };
    }
}
