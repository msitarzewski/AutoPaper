using System.Globalization;
using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace AutoPaper.Views.Panes;

/// <summary>Settings › Budget: the monthly cap (No limit · $1 · $2 · $5 · $10 · $20 · Custom), this month's estimated
/// spend, the estimated cost per wallpaper and per month at the current cadence, and the price table's date.</summary>
public sealed partial class BudgetPage : Page
{
    /// <summary>Choices in cents; null = no limit; 0 = custom.</summary>
    private static readonly uint?[] Choices = [null, 100, 200, 500, 1000, 2000, 0];

    private bool loading = true;

    public BudgetPage()
    {
        InitializeComponent();
        foreach (var cents in Choices)
        {
            BudgetBox.Items.Add(cents switch
            {
                null => Loc.Get("Budget_NoLimitChoice"),
                0 => Loc.Get("Budget_Custom"),
                { } value => Text.Dollars0(value),
            });
        }
        // The price table's date from the core (prices_as_of, "YYYY-MM-DD", pinned by CoreContractTests), in the
        // person's date format; read as local midnight so it's the same day in every time zone.
        var pricesAsOf = AutopaperCoreMethods.PricesAsOf();
        PricesLine.Text = Loc.Format("Budget_PricesAsOf",
            DateTimeOffset.TryParseExact(pricesAsOf, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal, out var asOf)
                ? Text.Date(asOf)
                : pricesAsOf);
    }

    private static AppModel Model => App.Model;

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        PaneHeader.Attach(Crumbs, "Budget", Frame);
        if (!Model.IsReady)
        {
            return;
        }
        ShowBudget();
        loading = false;
        await ShowSpendAsync();
    }

    /// <summary>The saved budget in the controls: on opening, and after a change that didn't save.</summary>
    private void ShowBudget()
    {
        var was = loading;
        loading = true;
        var budget = Model.Settings.MonthlyBudgetCents;
        var index = Array.IndexOf(Choices, budget);
        if (index < 0 || budget == 0)
        {
            index = Choices.Length - 1;
        }
        BudgetBox.SelectedIndex = index;
        CustomBudgetCard.Visibility = index == Choices.Length - 1 ? Visibility.Visible : Visibility.Collapsed;
        CustomBudget.Value = budget is { } cents ? cents / 100.0 : 5;
        loading = was;
    }

    private async Task ShowSpendAsync()
    {
        try
        {
            var spend = await Model.Call(engine => engine.SpendSummary());
            SpentCard.Description = Loc.Format("Budget_SpentDetail", Text.Budget(spend), Text.Count(spend.Images, "Wallpaper"));
            var cadence = Model.Settings.Cadence;
            EstimateCard.Description = spend.PerImageMicrousd == 0
                ? Loc.Get("Budget_EstimateFree")
                : cadence == Core.Cadence.Manual
                    ? Loc.Format("Budget_EstimateManual", Text.Money(spend.PerImageMicrousd))
                    : Loc.Format("Budget_EstimateDetail",
                        Text.Money(spend.PerImageMicrousd),
                        Text.Money(spend.MonthlyEstimateMicrousd),
                        Text.Cadence(cadence).ToLower(CultureInfo.CurrentCulture));
        }
        catch (Exception error)
        {
            SpentCard.Description = Text.Error(error, Model.Settings);
        }
    }

    /// <summary>Saves the budget; one that didn't save is said on the card it was changed on.</summary>
    private async Task SaveAsync(SettingsCard card, uint? cents)
    {
        try
        {
            await Model.UpdateSettingsAsync(settings => settings with { MonthlyBudgetCents = cents });
            PaneHeader.ClearProblem(card);
            await ShowSpendAsync();
        }
        catch (Exception error)
        {
            ShowBudget();
            PaneHeader.ShowProblem(card, error, Text.Error(error, Model.Settings));
        }
    }

    private async void OnBudgetChanged(object sender, SelectionChangedEventArgs e)
    {
        if (BudgetBox.SelectedIndex < 0)
        {
            return;
        }
        var choice = Choices[BudgetBox.SelectedIndex];
        CustomBudgetCard.Visibility = choice == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (loading)
        {
            return;
        }
        if (choice == 0)
        {
            await SaveAsync(BudgetCard, CustomCents());
        }
        else
        {
            await SaveAsync(BudgetCard, choice);
        }
    }

    private async void OnCustomBudgetChanged(NumberBox sender, NumberBoxValueChangedEventArgs args)
    {
        if (!loading && !double.IsNaN(args.NewValue))
        {
            await SaveAsync(CustomBudgetCard, CustomCents());
        }
    }

    private uint CustomCents()
    {
        var dollars = double.IsNaN(CustomBudget.Value) ? 5 : Math.Clamp(CustomBudget.Value, 1, 1000);
        return (uint)Math.Round(dollars * 100);
    }
}
