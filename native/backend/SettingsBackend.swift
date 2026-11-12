import Foundation

/// Builds HP PJL setting and inquiry jobs. Transport and response handling are separate:
/// Apple's CUPS USB backend accepts one-way jobs and does not expose PJL replies here.
public enum P1100SettingsBackend {
    public enum Page {
        case cleaning
        case demo
        case configuration
        case supplyStatus
    }

    public enum Setting: Equatable {
        case quietMode(Bool)
        case powerSaveTime(Int)
        case inactiveOffTime(Int)

        fileprivate var variable: String {
            switch self {
            case .quietMode: "QUIETMODE"
            case .powerSaveTime: "POWERSAVETIME"
            case .inactiveOffTime: "INACTIVEOFFTIME"
            }
        }

        fileprivate var value: String? {
            switch self {
            case let .quietMode(enabled): enabled ? "ON" : "OFF"
            case let .powerSaveTime(minutes): String(minutes)
            case let .inactiveOffTime(minutes): String(minutes)
            }
        }
    }

    private static let powerSaveTimes: Set<Int> = [1, 5, 15, 30, 60, 120, 0]
    private static let inactiveOffTimes: Set<Int> = [0, 30, 60, 120, 240, 480, 1440]

    /// Returns a complete PJL job suitable for the existing `p1100 send` raw USB transport.
    public static func setJob(for setting: Setting) throws -> Data {
        try validate(setting)
        return Data("\u{001B}%-12345X@PJL SET \(setting.variable)=\(setting.value!)\n\u{001B}%-12345X".utf8)
    }

    /// Returns the complete HP PJL job for a printer information page.
    public static func pageJob(for page: Page) -> Data {
        let command: String
        switch page {
        case .cleaning: command = "CLEANPRINTER"
        case .demo: command = "SET LANG=\n@PJL SET TESTPAGE=DEMOPAGE"
        case .configuration: command = "SET LANG=\n@PJL SET TESTPAGE=SELFTEST"
        case .supplyStatus: command = "SET LANG=\n@PJL SET TESTPAGE=SUPPLIES"
        }
        return Data("\u{001B}%-12345X@PJL \(command)\n\u{001B}%-12345X".utf8)
    }

    /// Returns an inquiry job. Sending it does not make the response available through the
    /// current CUPS USB transport; callers must not treat successful submission as readback.
    public static func inquiryJob(for setting: Setting) -> Data {
        Data("\u{001B}%-12345X@PJL INQUIRE \(setting.variable)\n\u{001B}%-12345X".utf8)
    }

    private static func validate(_ setting: Setting) throws {
        switch setting {
        case .quietMode:
            return
        case let .powerSaveTime(minutes) where powerSaveTimes.contains(minutes):
            return
        case let .inactiveOffTime(minutes) where inactiveOffTimes.contains(minutes):
            return
        case .powerSaveTime, .inactiveOffTime:
            throw BackendError.unsupportedValue(setting)
        }
    }

    public enum BackendError: Error, Equatable {
        case unsupportedValue(Setting)
    }
}
