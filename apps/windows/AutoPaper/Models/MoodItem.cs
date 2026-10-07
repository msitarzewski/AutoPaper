using AutoPaper.Core;
using AutoPaper.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;

namespace AutoPaper.Models;

/// <summary>One row of the Moods list: its name, its keywords in short, and "Current" or a Use button.</summary>
internal sealed partial class MoodItem(Mood mood) : ObservableObject
{
    public Mood Mood
    {
        get;
        set
        {
            field = value;
            OnPropertyChanged(string.Empty); // Everything below derives from it.
        }
    } = mood;

    public string Id => Mood.Id;

    public string Name => Mood.Name;

    public bool IsActive => Mood.Active;

    /// <summary>"rain · beach · night · no people".</summary>
    public string Summary => MoodText.Summary(Mood.Keywords);

    /// <summary>The row's accessible name: "Rainy beach, current mood, rain, beach, night".</summary>
    public string SpokenName => MoodText.RowName(Mood);

    /// <summary>The row's Use button, named after its mood ("Use Rainy beach").</summary>
    public string UseName => Loc.Format("Mood_UseName", Mood.Name);

    public Visibility CurrentVisibility => Mood.Active ? Visibility.Visible : Visibility.Collapsed;

    public Visibility UseVisibility => Mood.Active ? Visibility.Collapsed : Visibility.Visible;
}
