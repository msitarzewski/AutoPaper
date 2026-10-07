using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;

namespace AutoPaper.Controls;

/// <summary>
/// A clickable SettingsCard that UI Automation can invoke. The Toolkit's SettingsCardAutomationPeer (8.2.251219)
/// offers itself as the Invoke pattern without implementing IInvokeProvider, so Narrator's default action, Voice
/// Access and other assistive tools fail on clickable cards; this peer implements it. Pages handle
/// <see cref="Invoked"/> (raised by a click, Enter, Space, or UI Automation).
/// </summary>
public sealed partial class ActionCard : SettingsCard
{
    public ActionCard()
    {
        IsClickEnabled = true;
        Click += (sender, args) => Invoked?.Invoke(this, EventArgs.Empty);
    }

    public event EventHandler? Invoked;

    protected override AutomationPeer OnCreateAutomationPeer() => new Peer(this);

    private sealed partial class Peer(ActionCard owner) : SettingsCardAutomationPeer(owner), IInvokeProvider
    {
        public void Invoke()
        {
            if (owner.IsEnabled)
            {
                owner.Invoked?.Invoke(owner, EventArgs.Empty);
            }
        }
    }
}
