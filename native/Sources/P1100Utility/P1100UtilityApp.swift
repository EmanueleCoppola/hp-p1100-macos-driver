import SwiftUI
import AppKit
import ApplicationServices
import P1100SettingsBackend

@main
struct P1100UtilityApp: App {
    init() { PrinterAssociation.shared.register() }

    var body: some Scene {
        WindowGroup("P1100 Utility") { UtilityWindow().frame(minWidth: 690, minHeight: 400) }
            .defaultSize(width: 740, height: 440)
    }
}

struct UtilityWindow: View {
    @State private var queue = ""
    @State private var verifiedQueue: String?
    @State private var sleep = -1
    @State private var autoOff = -1
    @State private var originalValues: EcoSMARTValues?
    @State private var sleepEdited = false
    @State private var autoOffEdited = false
    @State private var readingSettings = false
    @State private var inquiryID = 0
    @State private var status = "Current printer settings cannot be read yet. Select only the settings you want to change."
    @State private var submitting = false
    @State private var showingCleaningConfirmation = false

    private let sleepOptions = [(0, "Never"), (1, "1 minute"), (5, "5 minutes"), (15, "15 minutes"), (30, "30 minutes"), (60, "1 hour"), (120, "2 hours")]
    private let offOptions = [(0, "Never"), (30, "30 minutes"), (60, "1 hour"), (120, "2 hours"), (240, "4 hours"), (480, "8 hours"), (1440, "24 hours")]

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                Text("Printer queue:")
                TextField("Printer queue", text: Binding(get: { queue }, set: updateQueue)).textFieldStyle(.roundedBorder).labelsHidden()
                Button("Refresh") { refreshSettings() }
                    .disabled(queue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || readingSettings || submitting)
            }
            HStack(alignment: .top, spacing: 18) {
                VStack(alignment: .leading, spacing: 16) {
                    GroupBox("Print Information Pages") {
                        VStack(alignment: .leading, spacing: 8) {
                            Button("Cleaning Page") { showingCleaningConfirmation = true }.disabled(!canSubmit)
                            Button("Demo Page") { printPage(.demo) }.disabled(!canSubmit)
                            Button("Configuration Page") { printPage(.configuration) }.disabled(!canSubmit)
                            Button("Supply Status Page") { printPage(.supplyStatus) }.disabled(!canSubmit)
                        }.frame(maxWidth: .infinity, alignment: .leading).padding(8)
                    }
                    GroupBox("Quiet Mode") {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Current: Unknown")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Text("Unavailable on HP LaserJet P1102")
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                        .padding(8)
                    }
                }.frame(maxWidth: .infinity)
                GroupBox("EcoSMART") {
                    VStack(alignment: .leading, spacing: 16) {
                        timerPicker("Sleep after", selection: Binding(get: { sleep }, set: { sleep = $0; sleepEdited = true }), current: originalValues?.powerSaveTime, options: sleepOptions)
                        timerPicker("Auto Off after", selection: Binding(get: { autoOff }, set: { autoOff = $0; autoOffEdited = true }), current: originalValues?.inactiveOffTime, options: offOptions)
                        Spacer(minLength: 0)
                    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).padding(8)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }.frame(maxHeight: .infinity)
            Text(status).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Close") { NSApplication.shared.terminate(nil) }
                Button("Apply") { apply() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit || readingSettings || !hasChanges)
            }
        }
        .padding(24)
        .alert("Print Cleaning Page?", isPresented: $showingCleaningConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Print Cleaning Page") { printPage(.cleaning) }
        } message: {
            Text("This will send a cleaning page job to the selected printer.")
        }
        .onAppear { updateAssociation(PrinterAssociation.shared.currentQueue) }
        .onReceive(NotificationCenter.default.publisher(for: .printerAssociationResolved)) { notification in
            updateAssociation(notification.object as? String)
        }
    }

    private func timerPicker(_ label: String, selection: Binding<Int>, current: Int?, options: [(Int, String)]) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack {
                Text(label)
                Spacer()
                Text(current.map { "Current: \(timerTitle($0, options: options))" } ?? "Current: Unknown")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Picker(label, selection: selection) {
                if current == nil { Text("Unknown — refresh settings").tag(-1) }
                if let current, !options.contains(where: { $0.0 == current }) {
                    Text("Current value (\(current))").tag(current)
                }
                ForEach(options, id: \.0) { value, title in Text(title).tag(value) }
            }
            .labelsHidden()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var canSubmit: Bool {
        !submitting && !readingSettings && !queue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var hasChanges: Bool {
        guard let originalValues else { return false }
        return (sleepEdited && sleep != originalValues.powerSaveTime) ||
            (autoOffEdited && autoOff != originalValues.inactiveOffTime)
    }

    private func apply() {
        do {
            var jobs = [Data]()
            guard let originalValues else { return }
            if sleepEdited && sleep != originalValues.powerSaveTime { jobs.append(try P1100SettingsBackend.setJob(for: .powerSaveTime(sleep))) }
            if autoOffEdited && autoOff != originalValues.inactiveOffTime { jobs.append(try P1100SettingsBackend.setJob(for: .inactiveOffTime(autoOff))) }
            submit(jobs, progress: "Sending settings to the print queue…", success: "CUPS accepted the settings job. Their application on the printer has not been verified.", failure: "Could not send all settings")
        } catch { status = "Invalid setting: \(error.localizedDescription)" }
    }

    private func printPage(_ page: P1100SettingsBackend.Page) {
        let job = P1100SettingsBackend.pageJob(for: page)
        submit([job], progress: "Sending page to the print queue…", success: "CUPS accepted the page job. Printing has not been verified.", failure: "Could not send page")
    }

    private func submit(_ jobs: [Data], progress: String, success: String, failure: String) {
        let selectedQueue = queue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !selectedQueue.isEmpty, !submitting else { return }
        submitting = true
        status = progress
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                for job in jobs {
                    let process = Process()
                    process.executableURL = URL(fileURLWithPath: "/usr/bin/lp")
                    process.arguments = ["-d", selectedQueue, "-o", "raw"]
                    let input = Pipe(), output = Pipe()
                    process.standardInput = input
                    process.standardOutput = output
                    process.standardError = output
                    try process.run()
                    input.fileHandleForWriting.write(job)
                    try input.fileHandleForWriting.close()
                    let response = output.fileHandleForReading.readDataToEndOfFile()
                    process.waitUntilExit()
                    guard process.terminationStatus == 0 else {
                        throw NSError(domain: "P1100Utility", code: Int(process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: String(decoding: response, as: UTF8.self)])
                    }
                }
                DispatchQueue.main.async {
                    submitting = false
                    status = success
                }
            } catch {
                DispatchQueue.main.async {
                    submitting = false
                    status = "\(failure): \(error.localizedDescription)"
                }
            }
        }
    }

    private func updateAssociation(_ resolvedQueue: String?) {
        inquiryID += 1
        readingSettings = false
        queue = resolvedQueue ?? ""
        verifiedQueue = resolvedQueue
        clearSettings()
        if let resolvedQueue, !resolvedQueue.isEmpty {
            status = "Associated printer queue: \(resolvedQueue). Reading EcoSMART settings…"
            readSettings(for: resolvedQueue)
        } else {
            status = "No printer association was verified. Enter a CUPS queue name, then refresh to read its EcoSMART settings."
        }
    }

    private func updateQueue(_ value: String) {
        queue = value
        guard value != verifiedQueue else { return }
        inquiryID += 1
        readingSettings = false
        verifiedQueue = nil
        clearSettings()
        status = value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "No printer association was verified. Current EcoSMART values are unknown."
            : "Queue entered manually. Refresh to read its EcoSMART settings."
    }

    private func clearSettings() {
        originalValues = nil
        sleep = -1
        autoOff = -1
        sleepEdited = false
        autoOffEdited = false
    }

    private func refreshSettings() {
        let selectedQueue = queue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !selectedQueue.isEmpty else { return }
        readSettings(for: selectedQueue)
    }

    private func readSettings(for selectedQueue: String) {
        inquiryID += 1
        let requestID = inquiryID
        readingSettings = true
        status = "Reading EcoSMART settings for \(selectedQueue)…"
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let values = try inquire(queue: selectedQueue)
                DispatchQueue.main.async {
                    guard requestID == inquiryID,
                          selectedQueue == queue.trimmingCharacters(in: .whitespacesAndNewlines) else { return }
                    originalValues = values
                    if !sleepEdited { sleep = values.powerSaveTime }
                    if !autoOffEdited { autoOff = values.inactiveOffTime }
                    readingSettings = false
                    status = "EcoSMART settings read for queue \(selectedQueue)."
                }
            } catch {
                DispatchQueue.main.async {
                    guard requestID == inquiryID,
                          selectedQueue == queue.trimmingCharacters(in: .whitespacesAndNewlines) else { return }
                    originalValues = nil
                    readingSettings = false
                    status = "Could not read EcoSMART settings for \(selectedQueue): \(error.localizedDescription)"
                }
            }
        }
    }

    private func timerTitle(_ value: Int, options: [(Int, String)]) -> String {
        options.first(where: { $0.0 == value })?.1 ?? "\(value)"
    }

    private func inquire(queue: String) throws -> EcoSMARTValues {
        let executable = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/p1100-inquire")
        let process = Process()
        process.executableURL = executable
        process.arguments = ["--queue", queue]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        try process.run()
        let response = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let detail = String(decoding: response, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            throw NSError(domain: "P1100Utility.Inquire", code: Int(process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: detail.isEmpty ? "Helper exited with status \(process.terminationStatus)." : detail])
        }
        return try JSONDecoder().decode(EcoSMARTValues.self, from: response)
    }
}

private struct EcoSMARTValues: Decodable {
    let powerSaveTime: Int
    let inactiveOffTime: Int

    enum CodingKeys: String, CodingKey {
        case powerSaveTime = "POWERSAVETIME"
        case inactiveOffTime = "INACTIVEOFFTIME"
    }
}

private extension Notification.Name {
    static let printerAssociationResolved = Notification.Name("PrinterAssociationResolved")
}

private final class PrinterAssociation: NSObject {
    static let shared = PrinterAssociation()
    private(set) var currentQueue: String?

    func register() {
        currentQueue = Self.queuePointingToThisUtility()
        NSAppleEventManager.shared().setEventHandler(
            self,
            andSelector: #selector(handleOpenApplication(_:withReplyEvent:)),
            forEventClass: AEEventClass(0x61657674), // aevt
            andEventID: AEEventID(0x6f617070) // oapp
        )
    }

    @objc private func handleOpenApplication(_ event: NSAppleEventDescriptor, withReplyEvent reply: NSAppleEventDescriptor) {
        let printerID = event.paramDescriptor(forKeyword: AEKeyword(0x70726474))?.stringValue // prdt
        let queue = printerID.flatMap(Self.cupsDestination(forPrinterID:)) ?? Self.queuePointingToThisUtility()
        currentQueue = queue
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .printerAssociationResolved, object: queue)
        }
    }

    private static func queuePointingToThisUtility() -> String? {
        let directory = URL(fileURLWithPath: "/etc/cups/ppd", isDirectory: true)
        guard let files = try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil) else { return nil }
        let expected = "*APPrinterUtilityPath: \"\(Bundle.main.bundleURL.path)\""
        let queues = files.filter { file in
            guard file.pathExtension == "ppd", let contents = try? String(contentsOf: file, encoding: .isoLatin1) else { return false }
            return contents.components(separatedBy: .newlines).contains(expected)
        }.map { $0.deletingPathExtension().lastPathComponent }
        return queues.count == 1 ? queues[0] : nil
    }

    private static func cupsDestination(forPrinterID printerID: String) -> String? {
        guard let printer = PMPrinterCreateFromPrinterID(printerID as CFString) else { return nil }
        defer { PMRelease(UnsafeRawPointer(printer)) }
        guard let printerID = PMPrinterGetID(printer) as String? else { return nil }

        let check = Process()
        check.executableURL = URL(fileURLWithPath: "/usr/bin/lpstat")
        check.arguments = ["-p", printerID]
        check.standardOutput = FileHandle.nullDevice
        check.standardError = FileHandle.nullDevice
        do {
            try check.run()
            check.waitUntilExit()
            return check.terminationStatus == 0 ? printerID : nil
        } catch {
            return nil
        }
    }
}
