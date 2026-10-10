import AutopaperCore
import Foundation
import FoundationModels

/// Apple's on-device model (Apple Intelligence), for the core's "On this Mac" writer. The core calls these methods from
/// a blocking thread, so `compose` waits for the model there; it never runs on the main thread.
final class AppleSystemModel: SystemModel {
    func status() -> SystemModelStatus {
        let model = SystemLanguageModel.default
        switch model.availability {
        case .available:
            return SystemModelStatus(available: true, reason: nil, name: "Apple Intelligence", contextTokens: UInt32(model.contextSize))
        case .unavailable(let reason):
            let why: SystemModelReason
            switch reason {
            case .deviceNotEligible: why = .deviceNotEligible
            case .appleIntelligenceNotEnabled: why = .notEnabled
            case .modelNotReady: why = .notReady
            @unknown default: why = .other
            }
            return SystemModelStatus(available: false, reason: why, name: "Apple Intelligence", contextTokens: 0)
        }
    }

    func compose(system: String, user: String, schemaJson: String, temperature: Float, maxOutputTokens: UInt32) -> SystemComposeOutcome {
        let current = status()
        guard current.available else {
            return Self.outcome(problem: .unavailable(reason: current.reason ?? .other))
        }
        let box = ResultBox()
        let semaphore = DispatchSemaphore(value: 0)
        Task.detached {
            box.value = await Self.respond(system: system, user: user, schemaJson: schemaJson,
                                           temperature: temperature, maxOutputTokens: maxOutputTokens)
            semaphore.signal()
        }
        semaphore.wait()
        return box.value ?? Self.outcome(problem: .failed(detail: "The on-device model gave no answer."))
    }

    private static func respond(system: String, user: String, schemaJson: String, temperature: Float,
                                maxOutputTokens: UInt32) async -> SystemComposeOutcome {
        do {
            let schema = try JSONDecoder().decode(GenerationSchema.self, from: Data(schemaJson.utf8))
            let session = LanguageModelSession(model: .default, instructions: system)
            // The model takes 0 to 1; the core's temperatures run to 2.
            let options = GenerationOptions(temperature: Double(min(1, max(0, temperature))),
                                            maximumResponseTokens: Int(maxOutputTokens))
            let answer = try await session.respond(to: user, schema: schema, options: options)
            let json = answer.content.jsonString
            // Apple doesn't report token counts from a structured answer; about four characters to a token.
            return SystemComposeOutcome(json: json, inputTokens: UInt64((system.count + user.count) / 4),
                                        outputTokens: UInt64(json.count / 4), problem: nil)
        } catch let error as LanguageModelSession.GenerationError {
            switch error {
            case .exceededContextWindowSize: return outcome(problem: .contextTooSmall)
            case .guardrailViolation, .refusal: return outcome(problem: .refused)
            case .rateLimited: return outcome(problem: .rateLimited)
            case .assetsUnavailable: return outcome(problem: .unavailable(reason: .notReady))
            default: return outcome(problem: .failed(detail: String(describing: error)))
            }
        } catch {
            return outcome(problem: .failed(detail: String(describing: error)))
        }
    }

    private static func outcome(problem: SystemModelProblem) -> SystemComposeOutcome {
        SystemComposeOutcome(json: "", inputTokens: 0, outputTokens: 0, problem: problem)
    }
}

/// Carries the answer out of the task that wrote it (one write, then the semaphore, then one read).
private final class ResultBox: @unchecked Sendable {
    var value: SystemComposeOutcome?
}
