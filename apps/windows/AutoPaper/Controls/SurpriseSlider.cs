using System.Globalization;
using AutoPaper.Services;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace AutoPaper.Controls;

/// <summary>
/// The stock Slider (0–100), whose UI Automation value reads as words: "35 percent, fresh" (spec: Surprise speaks
/// its value and band), and whose help text is the band's description ("Believable scenes, each with one unexpected
/// choice…"), so both are heard when it gets focus. UIA's RangeValue pattern carries only the number, so the peer adds
/// the Value pattern with the words, as browsers do for aria-valuetext; Narrator reads it.
/// </summary>
public sealed partial class SurpriseSlider : Slider
{
    public SurpriseSlider()
    {
        ValueChanged += OnValueChanged;
    }

    public string SpokenValue => Text.SurpriseSpoken((int)Math.Round(Value));

    /// <summary>Sets the band's description as the slider's help text, telling UI Automation clients it changed.</summary>
    public void SetBandDescription(string description)
    {
        var before = AutomationProperties.GetHelpText(this);
        if (before == description)
        {
            return;
        }
        AutomationProperties.SetHelpText(this, description);
        FrameworkElementAutomationPeer.FromElement(this)?.RaisePropertyChangedEvent(AutomationElementIdentifiers.HelpTextProperty, before, description);
    }

    protected override AutomationPeer OnCreateAutomationPeer() => new Peer(this);

    private void OnValueChanged(object sender, RangeBaseValueChangedEventArgs args)
    {
        if (FrameworkElementAutomationPeer.FromElement(this) is Peer peer)
        {
            var before = Text.SurpriseSpoken((int)Math.Round(args.OldValue));
            peer.RaisePropertyChangedEvent(ValuePatternIdentifiers.ValueProperty, before, SpokenValue);
        }
    }

    private sealed partial class Peer(SurpriseSlider owner) : SliderAutomationPeer(owner), IValueProvider
    {
        // The slider can be changed, so its Value pattern isn't read-only (screen readers would say "read only").
        // Explicit: RangeValue's IsReadOnly and Value (the number) stay as they are.
        bool IValueProvider.IsReadOnly => !owner.IsEnabled;

        string IValueProvider.Value => owner.SpokenValue;

        /// <summary>A value set as text ("62", "62 percent", "62 percent, adventurous"): its leading number.</summary>
        void IValueProvider.SetValue(string value)
        {
            if (!owner.IsEnabled)
            {
                throw new ElementNotEnabledException();
            }
            var digits = new string((value ?? "").TrimStart().TakeWhile(c => char.IsAsciiDigit(c) || c == '.').ToArray());
            if (!double.TryParse(digits, NumberStyles.AllowDecimalPoint, CultureInfo.InvariantCulture, out var number))
            {
                throw new ArgumentException("Surprise takes a number from 0 to 100.", nameof(value));
            }
            owner.Value = Math.Clamp(number, owner.Minimum, owner.Maximum);
        }

        protected override object GetPatternCore(PatternInterface patternInterface) =>
            patternInterface == PatternInterface.Value ? this : base.GetPatternCore(patternInterface);
    }
}
