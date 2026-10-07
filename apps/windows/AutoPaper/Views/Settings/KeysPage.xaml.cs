using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using AutoPaper.Views;
using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace AutoPaper.Views.Panes;

/// <summary>
/// Settings › Keys: the OpenAI and Google Gemini API keys, and a key for each OpenAI-compatible server in use
/// (account from secret_account_for: "Key for &lt;address&gt;"). Secure fields that save as you type to Windows
/// Credential Manager ("AutoPaper:&lt;account&gt;"); clearing a field deletes the key. Opened from a link such as
/// "Add your OpenAI key" (docs/app-spec.md 6a), it focuses that key's field.
/// </summary>
public sealed partial class KeysPage : Page, IDefaultFocus
{
    private readonly Dictionary<PasswordBox, DispatcherTimerLite> pending = [];
    private readonly Dictionary<PasswordBox, TextBlock> statuses = [];
    private bool loading = true;
    /// <summary>The account whose field gets focus (a link to it), or null for the first field.</summary>
    private string? focusAccount;

    public KeysPage()
    {
        InitializeComponent();
        // Accounts from the core (secret_account_for), never copied here.
        OpenAiKey.Tag = AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.OpenAi, "", null));
        GoogleKey.Tag = AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.Google, "", null));
    }

    private static AppModel Model => App.Model;

    /// <summary>The field a link asked for, else the first key field (not the "Get a key" link before it).</summary>
    public UIElement DefaultFocusElement => BoxFor(focusAccount) ?? OpenAiKey;

    private PasswordBox? BoxFor(string? account) => account is null ? null
        : new[] { OpenAiKey, GoogleKey }.Concat(CompatibleKeys.Children.OfType<SettingsCard>().Select(card => card.Content).OfType<PasswordBox>())
            .FirstOrDefault(box => box.Tag as string == account);

    /// <summary>Focuses a key's field (a link such as "Add your OpenAI key" while this page is open, or just opened).</summary>
    public void FocusKey(string account)
    {
        focusAccount = account;
        if (IsLoaded && !loading && BoxFor(account) is { } box)
        {
            box.Focus(FocusState.Keyboard);
            box.StartBringIntoView();
        }
    }

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        PaneHeader.Attach(Crumbs, "Keys", Frame);
        if (e.Parameter is string account)
        {
            focusAccount = account;
        }
        loading = true;
        await LoadAsync(OpenAiKey, OpenAiStatus);
        await LoadAsync(GoogleKey, GoogleStatus);
        if (Model.IsReady)
        {
            AddCompatibleCards();
        }
        loading = false;
        if (focusAccount is { } wanted)
        {
            if (!IsLoaded)
            {
                await PageFocus.FocusAsync(this, FocusState.Keyboard);
            }
            else
            {
                FocusKey(wanted);
            }
        }
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        // Save anything still waiting for the typing pause.
        foreach (var (box, timer) in pending.Where(entry => entry.Value.Pending).ToList())
        {
            timer.Stop();
            _ = SaveAsync(box);
        }
    }

    private async Task LoadAsync(PasswordBox box, TextBlock status)
    {
        statuses[box] = status;
        var account = (string)box.Tag;
        var saved = await Task.Run(() => Model.Secrets.Get(account));
        box.Password = saved ?? "";
        status.Text = Loc.Get(saved is null ? "Key_NotSet" : "Key_Saved");
    }

    /// <summary>A card per distinct OpenAI-compatible server among the two providers.</summary>
    private void AddCompatibleCards()
    {
        CompatibleKeys.Children.Clear();
        var settings = Model.Settings;
        var servers = new[] { settings.TextProvider, settings.ImageProvider }
            .Where(selection => selection.Kind == ProviderKind.OpenAiCompatible)
            .Select(selection => (Selection: selection, Account: AutopaperCoreMethods.SecretAccountFor(selection)))
            .Where(entry => entry.Account is not null)
            .DistinctBy(entry => entry.Account)
            .ToList();
        CompatibleNone.Visibility = servers.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        foreach (var (selection, account) in servers)
        {
            var address = selection.BaseUrl ?? "";
            var label = Loc.Format("Key_ForServer", address);
            var status = new TextBlock { TextWrapping = TextWrapping.Wrap };
            var box = new PasswordBox { Width = 300, Tag = account };
            AutomationProperties.SetName(box, label);
            box.PasswordChanged += OnKeyChanged;
            var card = new SettingsCard
            {
                Header = label,
                Description = new StackPanel
                {
                    Spacing = 2,
                    Children =
                    {
                        status,
                        new TextBlock
                        {
                            Text = Loc.Get("Key_ServerOptional"),
                            TextWrapping = TextWrapping.Wrap,
                            Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
                        },
                    },
                },
                HeaderIcon = new FontIcon { Glyph = "\uE8D7" },
                Content = box,
            };
            AutomationProperties.SetName(card, label);
            CompatibleKeys.Children.Add(card);
            _ = LoadAsync(box, status);
        }
    }

    private void OnKeyChanged(object sender, RoutedEventArgs e)
    {
        if (loading || sender is not PasswordBox box)
        {
            return;
        }
        if (!pending.TryGetValue(box, out var timer))
        {
            timer = new DispatcherTimerLite(DispatcherQueue, TimeSpan.FromMilliseconds(600), () => SaveAsync(box));
            pending[box] = timer;
        }
        timer.Restart();
    }

    private async Task SaveAsync(PasswordBox box)
    {
        var account = (string)box.Tag;
        var value = box.Password;
        statuses.TryGetValue(box, out var status);
        try
        {
            await Task.Run(() => Model.Secrets.Set(account, value));
            var message = Loc.Get(value.Trim().Length == 0 ? "Key_Removed" : "Key_Saved");
            if (status is not null)
            {
                status.Text = message;
            }
            Problem.Visibility = Visibility.Collapsed;
            Model.Announce(Loc.Format("Key_SavedSpoken", AutomationProperties.GetName(box), message));
            // Views that need this key refresh themselves (Providers lists models again; Now's "Add your … key" goes).
            Model.KeySaved(account, value.Trim().Length > 0);
        }
        catch (Exception error)
        {
            // Windows' own message (or the store's) goes to the log; the window says it in the app's words.
            Log.Error("Saving a key", error);
            Problem.Text = Loc.Get(error is ArgumentException ? "Key_TooLong" : "Key_SaveFailed");
            Problem.Visibility = Visibility.Visible;
            Model.AnnounceError(Problem.Text);
        }
    }
}
