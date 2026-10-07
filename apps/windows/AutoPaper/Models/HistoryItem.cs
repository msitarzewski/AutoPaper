using AutoPaper.Core;
using AutoPaper.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media.Imaging;

namespace AutoPaper.Models;

/// <summary>One wallpaper in History: its thumbnail, title, date, badges and spoken name.</summary>
internal sealed partial class HistoryItem : ObservableObject
{
    public HistoryItem(Generation generation, string description, bool hasEchoes)
    {
        Generation = generation;
        Description = description;
        HasEchoes = hasEchoes;
    }

    public Generation Generation
    {
        get;
        set
        {
            SetProperty(ref field, value);
            OnPropertyChanged(string.Empty); // Everything below derives from it.
        }
    }

    /// <summary>describe(id): title, summary and echo note as sentences.</summary>
    public string Description
    {
        get;
        set
        {
            SetProperty(ref field, value);
            OnPropertyChanged(nameof(SpokenName));
        }
    }

    /// <summary>has_echoes(id): another wallpaper in History is its original or one of its echoes (History then
    /// offers "Show original and echoes").</summary>
    public bool HasEchoes { get; set; }

    public string Id => Generation.Id;

    public string Title => Generation.Concept.Title;

    public string When => Text.WhenMade(Generation.CreatedAt);

    public Rating Rating => Generation.Rating;

    public bool IsEcho => Generation.EchoOf is not null;

    /// <summary>The item's accessible name: describe(id), its rating in words, and when it was made.</summary>
    public string SpokenName => $"{Text.Spoken(Description, Rating)} {When}.";

    public string MoreName => Loc.Format("History_MoreName", Title);

    public Visibility LikedBadge => Rating == Rating.Liked ? Visibility.Visible : Visibility.Collapsed;

    public Visibility DislikedBadge => Rating == Rating.Disliked ? Visibility.Visible : Visibility.Collapsed;

    public Visibility EchoBadge => IsEcho ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>"Original" or "Echo" in the lineage list.</summary>
    public string LineageRole => Loc.Get(IsEcho ? "Lineage_Echo" : "Lineage_Original");

    public string EchoNote => Generation.EchoNote ?? "";

    public BitmapImage? Thumbnail
    {
        get;
        private set => SetProperty(ref field, value);
    }

    private string? thumbnailFor;

    /// <summary>Loads the 640 px thumbnail, decoded at the size shown.</summary>
    public async Task LoadThumbnailAsync(int decodeWidth)
    {
        var path = Generation.ThumbPath;
        if (path is null || path == thumbnailFor || !File.Exists(path))
        {
            return;
        }
        thumbnailFor = path;
        try
        {
            var bitmap = new BitmapImage { DecodePixelWidth = decodeWidth };
            using var stream = File.OpenRead(path);
            await bitmap.SetSourceAsync(stream.AsRandomAccessStream());
            Thumbnail = bitmap;
        }
        catch (Exception)
        {
            thumbnailFor = null; // Try again next time it's shown.
        }
    }
}
