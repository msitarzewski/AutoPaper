using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.System;

namespace AutoPaper.Views;

/// <summary>
/// First run: add a few keywords, choose who writes and paints (OpenAI · Google Gemini · Local · Try it without
/// AI), add a key if needed, then Make my first wallpaper. Skippable; shown once (Preferences.FirstRunDone).
/// </summary>
public sealed partial class FirstRunPage : Page, IDefaultFocus
{
    private readonly List<string> added = [];
    private bool making;

    public FirstRunPage()
    {
        InitializeComponent();
        ShowStep3();
    }

    private static AppModel Model => App.Model;

    /// <summary>The first step: the keyword field.</summary>
    public UIElement DefaultFocusElement => KeywordBox;

    /// <summary>Where the core keeps a hosted provider's key (secret_account_for).</summary>
    private static string KeyAccount(ProviderKind kind) =>
        AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(kind, "", null)) ?? throw new InvalidOperationException($"{kind} takes no key.");

    private enum Choice
    {
        OpenAi,
        Gemini,
        Local,
        Demo,
    }

    private Choice Chosen => (Choice)Math.Max(0, Who.SelectedIndex);

    private async void OnKeywordKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            e.Handled = true;
            await AddKeywordAsync();
        }
    }

    private async void OnAddKeyword(object sender, RoutedEventArgs e) => await AddKeywordAsync();

    private async Task AddKeywordAsync()
    {
        var text = KeywordItem.Normalise(KeywordBox.Text);
        if (text.Length == 0 || !Model.IsReady)
        {
            return;
        }
        try
        {
            var keyword = await Model.Call(engine => engine.AddKeyword(text, KeywordWeight.Must));
            if (!added.Contains(keyword.Text, StringComparer.CurrentCultureIgnoreCase))
            {
                added.Add(keyword.Text);
            }
            KeywordBox.Text = "";
            AddedLine.Text = Loc.Format("FirstRun_Added", string.Join(", ", added));
            AddedLine.Visibility = Visibility.Visible;
            Model.Announce(Loc.Format("Keyword_Added", keyword.Text));
        }
        catch (Exception error)
        {
            ShowProblem(Text.Error(error, Model.Settings));
        }
    }

    private void OnWhoChanged(object sender, SelectionChangedEventArgs e) => ShowStep3();

    private void OnKeyChanged(object sender, RoutedEventArgs e) => UpdateMakeButton();

    private void ShowStep3()
    {
        if (Step3Title is null)
        {
            return; // Still loading the page.
        }
        var choice = Chosen;
        var hosted = choice is Choice.OpenAi or Choice.Gemini;
        Step3Title.Text = Loc.Get(choice switch
        {
            Choice.OpenAi or Choice.Gemini => "FirstRun_Step3Key",
            Choice.Local => "FirstRun_Step3Local",
            _ => "FirstRun_Step3Demo",
        });
        Step3Body.Text = Loc.Get(choice switch
        {
            Choice.OpenAi => "FirstRun_Step3OpenAiBody",
            Choice.Gemini => "FirstRun_Step3GeminiBody",
            Choice.Local => "FirstRun_Step3LocalBody",
            _ => "FirstRun_Step3DemoBody",
        });
        KeyStep.Visibility = hosted ? Visibility.Visible : Visibility.Collapsed;
        LocalStep.Visibility = choice == Choice.Local ? Visibility.Visible : Visibility.Collapsed;
        GeminiNoteLine.Visibility = choice == Choice.Gemini ? Visibility.Visible : Visibility.Collapsed;
        if (hosted)
        {
            var label = Loc.Get(choice == Choice.OpenAi ? "FirstRun_OpenAiKey" : "FirstRun_GeminiKey");
            KeyBox.Header = label;
            AutomationProperties.SetName(KeyBox, label);
            GetKeyLink.Content = Loc.Get(choice == Choice.OpenAi ? "FirstRun_GetOpenAiKey" : "FirstRun_GetGeminiKey");
            GetKeyLink.NavigateUri = new Uri(choice == Choice.OpenAi ? "https://platform.openai.com/api-keys" : "https://aistudio.google.com/apikey");
            KeyBox.Password = Model.Secrets.Get(KeyAccount(choice == Choice.OpenAi ? ProviderKind.OpenAi : ProviderKind.Google)) ?? "";
        }
        UpdateMakeButton();
    }

    /// <summary>OpenAI and Gemini need a key before the first wallpaper.</summary>
    private void UpdateMakeButton() =>
        MakeButton.IsEnabled = Chosen is not (Choice.OpenAi or Choice.Gemini) || KeyBox.Password.Trim().Length > 0;

    private async void OnMake(object sender, RoutedEventArgs e)
    {
        // The button stays enabled (disabling the focused button would throw focus to the title bar); a second press
        // while saving does nothing.
        if (!Model.IsReady || making)
        {
            return;
        }
        making = true;
        var choice = Chosen;
        var (writer, painter) = choice switch
        {
            Choice.OpenAi => (ProviderKind.OpenAi, ProviderKind.OpenAi),
            Choice.Gemini => (ProviderKind.Google, ProviderKind.Google),
            Choice.Local => (ProviderKind.Ollama, ProviderKind.ComfyUi),
            _ => (ProviderKind.Demo, ProviderKind.Demo),
        };
        try
        {
            if (choice is Choice.OpenAi or Choice.Gemini)
            {
                var account = KeyAccount(choice == Choice.OpenAi ? ProviderKind.OpenAi : ProviderKind.Google);
                var key = KeyBox.Password;
                await Task.Run(() => Model.Secrets.Set(account, key));
                Model.KeySaved(account, key.Trim().Length > 0);
            }
            await Model.UpdateSettingsAsync(settings => settings with
            {
                TextProvider = new ProviderSelection(writer, "", null),
                ImageProvider = new ProviderSelection(painter, "", null),
            });
        }
        catch (Exception error)
        {
            ShowProblem(Text.Error(error, Model.Settings));
            making = false;
            return;
        }
        App.Window.FinishFirstRun(makingOne: true);
        await Model.NewWallpaperAsync();
    }

    private void OnSkip(object sender, RoutedEventArgs e) => App.Window.FinishFirstRun(makingOne: false);

    private void ShowProblem(string message)
    {
        Problem.Text = message;
        Problem.Visibility = Visibility.Visible;
        Model.AnnounceError(message);
    }
}
