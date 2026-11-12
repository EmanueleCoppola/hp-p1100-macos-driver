import Foundation

private func check(_ setting: P1100SettingsBackend.Setting, _ expected: String) throws {
    let actual = try P1100SettingsBackend.setJob(for: setting)
    precondition(actual == Data(expected.utf8))
}

let start = "\u{001B}%-12345X@PJL "
let end = "\n\u{001B}%-12345X"
try check(.quietMode(true), start + "SET QUIETMODE=ON" + end)
try check(.quietMode(false), start + "SET QUIETMODE=OFF" + end)
for value in [1, 5, 15, 30, 60, 120, 0] {
    try check(.powerSaveTime(value), start + "SET POWERSAVETIME=\(value)" + end)
}
for value in [0, 30, 60, 120, 240, 480, 1440] {
    try check(.inactiveOffTime(value), start + "SET INACTIVEOFFTIME=\(value)" + end)
}
for (page, command) in [
    (P1100SettingsBackend.Page.cleaning, "CLEANPRINTER"),
    (.demo, "SET LANG=\n@PJL SET TESTPAGE=DEMOPAGE"),
    (.configuration, "SET LANG=\n@PJL SET TESTPAGE=SELFTEST"),
    (.supplyStatus, "SET LANG=\n@PJL SET TESTPAGE=SUPPLIES")
] {
    precondition(P1100SettingsBackend.pageJob(for: page) == Data((start + command + end).utf8))
}
for (setting, variable) in [
    (P1100SettingsBackend.Setting.quietMode(false), "QUIETMODE"),
    (.powerSaveTime(0), "POWERSAVETIME"),
    (.inactiveOffTime(0), "INACTIVEOFFTIME")
] {
    let query = String(decoding: P1100SettingsBackend.inquiryJob(for: setting), as: UTF8.self)
    precondition(query == start + "INQUIRE \(variable)\n\u{001B}%-12345X")
}
for setting in [P1100SettingsBackend.Setting.powerSaveTime(2), .inactiveOffTime(15)] {
    do {
        _ = try P1100SettingsBackend.setJob(for: setting)
        preconditionFailure("unsupported values must fail")
    } catch P1100SettingsBackend.BackendError.unsupportedValue {
        // Expected.
    }
}
print("settings backend checks passed")
