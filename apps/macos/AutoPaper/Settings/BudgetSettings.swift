import AutopaperCore
import SwiftUI

/// Budget: the monthly cap (No limit · $1 · $2 · $5 · $10 · $20 · Custom), this month's estimated spend, and the
/// estimated cost of one wallpaper and of a month at the current pace, with the date of the price table.
struct BudgetSettings: View {
    @Environment(AppModel.self) private var model
    @State private var custom = false
    @State private var customDollars: Double = 5
    @FocusState private var customFocused: Bool

    /// Picker tags: -1 = no limit, -2 = custom, else cents.
    private static let noLimit = -1
    private static let customTag = -2

    var body: some View {
        SettingsPane { settings in
            Section {
                Picker(selection: Binding(get: { tag(settings) }, set: { choose($0) })) {
                    ForEach(Choices.budgets, id: \.self) { cents in
                        Text(cents.map { Formatting.dollars(cents: $0) } ?? "No limit").tag(cents.map(Int.init) ?? Self.noLimit)
                    }
                    Text("Custom").tag(Self.customTag)
                } label: {
                    Text("Monthly budget")
                    Text("When a new wallpaper would go over it, AutoPaper keeps your current wallpaper and waits for next month.")
                }
                if custom || isCustom(settings) {
                    TextField("Custom budget", value: $customDollars, format: .currency(code: "USD").locale(Locale(identifier: "en_US")))
                        .focused($customFocused)
                        .onSubmit(saveCustom)
                        .onChange(of: customFocused) { _, focused in if !focused { saveCustom() } }
                        .onAppear { if let cents = settings.monthlyBudgetCents { customDollars = Double(cents) / 100 } }
                }
            }
            if let problem = model.budgetProblem {
                Section("New wallpapers are waiting") {
                    Text(problem.sentence)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if let spend = model.spend {
                // The amounts are what this pane is for, so they're in primary text: a grouped form's values are
                // secondary, which measures 3.9:1 in Light mode (WCAG 1.4.3 asks 4.5:1).
                Section {
                    LabeledContent("Spent") {
                        Text(Formatting.budgetLine(spentMicroUSD: spend.spentMicrousd, budgetCents: spend.budgetCents))
                            .foregroundStyle(.primary)
                    }
                    LabeledContent("Each wallpaper") {
                        Text(spend.perImageMicrousd == 0 ? "Free" : "about \(Formatting.dollars(microUSD: spend.perImageMicrousd))")
                            .foregroundStyle(.primary)
                    }
                    LabeledContent {
                        Text(monthly(spend, settings))
                            .foregroundStyle(.primary)
                    } label: {
                        Text("A month")
                        Text(settings.cadence == .manual
                             ? "Only the wallpapers you ask for."
                             : "With a new wallpaper \(settings.cadence.title.lowercased()).")
                    }
                    LabeledContent("Wallpapers made") {
                        Text(spend.images == 1 ? "1 this month" : "\(spend.images) this month")
                            .foregroundStyle(.primary)
                    }
                } header: {
                    Text("This month")
                } footer: {
                    Text("Estimated from the providers' published prices as of \(Formatting.pricesDate()). Local providers and the Demo cost nothing. Months follow UTC.")
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .task { await model.refreshStatus() }
    }

    private func monthly(_ spend: SpendSummary, _ settings: EngineSettings) -> String {
        if settings.cadence == .manual { return "Depends on you" }
        return spend.monthlyEstimateMicrousd == 0 ? "Free" : "about \(Formatting.dollars(microUSD: spend.monthlyEstimateMicrousd))"
    }

    private func isCustom(_ settings: EngineSettings) -> Bool {
        guard let cents = settings.monthlyBudgetCents else { return false }
        return !Choices.budgets.contains(cents)
    }

    private func tag(_ settings: EngineSettings) -> Int {
        if custom || isCustom(settings) { return Self.customTag }
        return settings.monthlyBudgetCents.map(Int.init) ?? Self.noLimit
    }

    private func choose(_ tag: Int) {
        switch tag {
        case Self.customTag:
            custom = true
            customDollars = Double(model.settings?.monthlyBudgetCents ?? 500) / 100
            customFocused = true
        case Self.noLimit:
            custom = false
            model.updateSettings { $0.monthlyBudgetCents = nil }
        default:
            custom = false
            model.updateSettings { $0.monthlyBudgetCents = UInt32(tag) }
        }
    }

    private func saveCustom() {
        let cents = UInt32(clamping: max(0, Int((customDollars * 100).rounded())))
        model.updateSettings { $0.monthlyBudgetCents = cents }
    }
}
