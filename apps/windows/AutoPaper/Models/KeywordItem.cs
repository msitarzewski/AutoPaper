using AutoPaper.Core;
using AutoPaper.Services;
using CommunityToolkit.Mvvm.ComponentModel;

namespace AutoPaper.Models;

/// <summary>One keyword row of a mood (Moods › detail).</summary>
internal sealed partial class KeywordItem : ObservableObject
{
    public KeywordItem(Keyword keyword)
    {
        Id = keyword.Id;
        Text = keyword.Text;
        Weight = keyword.Weight;
    }

    public string Id { get; }

    public string Text
    {
        get;
        set
        {
            if (SetProperty(ref field, value))
            {
                OnPropertyChanged(nameof(SpokenName));
                OnPropertyChanged(nameof(EditName));
                OnPropertyChanged(nameof(WeightName));
                OnPropertyChanged(nameof(RemoveName));
            }
        }
    }

    public KeywordWeight Weight
    {
        get;
        set
        {
            if (SetProperty(ref field, value))
            {
                OnPropertyChanged(nameof(WeightIndex));
                OnPropertyChanged(nameof(SpokenName));
            }
        }
    }

    /// <summary>The Must · Maybe · Avoid choice's index: Must 0, Maybe 1, Avoid 2.</summary>
    public int WeightIndex
    {
        get => Weight switch { KeywordWeight.Must => 0, KeywordWeight.Maybe => 1, _ => 2 };
        set => Weight = value switch { 0 => KeywordWeight.Must, 1 => KeywordWeight.Maybe, _ => KeywordWeight.Avoid };
    }

    /// <summary>The row's accessible name: "rain, Must".</summary>
    public string SpokenName => Loc.Format("Keyword_Spoken", Text, WeightWord(Weight));

    public string EditName => Loc.Format("Keyword_EditName", Text);

    public string WeightName => Loc.Format("Keyword_WeightName", Text);

    public string RemoveName => Loc.Format("Keyword_RemoveName", Text);

    public static string WeightWord(KeywordWeight weight) => Loc.Get($"Weight_{weight}");

    /// <summary>The engine's own normalising (trimmed, single-spaced), for finding a duplicate before adding.</summary>
    public static string Normalise(string text) => MoodText.Normalise(text);
}
