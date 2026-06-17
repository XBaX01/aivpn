import Foundation
import Combine

class LocalizationManager: ObservableObject {
    static let shared = LocalizationManager()

    private static let appVersion: String = {
        return Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?"
    }()

    @Published var language: String = "en" {
        didSet { UserDefaults.standard.set(language, forKey: "app_language") }
    }

    private let strings: [String: [String: String]] = [
        "status_connected":    ["en": "Connected",      "ru": "Подключено"],
        "status_disconnected": ["en": "Disconnected",   "ru": "Отключено"],
        "status_connecting":   ["en": "Connecting…",    "ru": "Подключение…"],
        "status_reconnecting": ["en": "Reconnecting…",  "ru": "Переподключение…"],
        "connect":             ["en": "Connect",        "ru": "Подключить"],
        "disconnect":          ["en": "Disconnect",     "ru": "Отключить"],
        "enter_key":           ["en": "Connection key (aivpn://…)", "ru": "Ключ подключения (aivpn://…)"],
        "full_tunnel":         ["en": "Full tunnel (route all traffic)", "ru": "Полный туннель (весь трафик)"],
        "add_key":             ["en": "Add Key",        "ru": "Добавить ключ"],
        "key_name":            ["en": "Key Name",       "ru": "Название ключа"],
        "save_key":            ["en": "Save",           "ru": "Сохранить"],
        "cancel":              ["en": "Cancel",         "ru": "Отмена"],
        "delete":              ["en": "Delete",         "ru": "Удалить"],
        "edit":                ["en": "Edit",           "ru": "Изменить"],
        "connection_keys":     ["en": "Saved Keys",     "ru": "Сохранённые ключи"],
        "no_keys_yet":         ["en": "No keys yet",   "ru": "Нет ключей"],
        "add_first_key":       ["en": "Add First Key",  "ru": "Добавить первый ключ"],
        "no_key_selected":     ["en": "No key selected","ru": "Ключ не выбран"],
        "duplicate_key":       ["en": "This key already exists", "ru": "Этот ключ уже существует"],
        "delete_key_confirm":  ["en": "Delete Key?",    "ru": "Удалить ключ?"],
        "delete_key_message":  ["en": "Are you sure you want to delete this key?",
                                "ru": "Вы уверены что хотите удалить этот ключ?"],
        "split_tunnel":        ["en": "Excluded Domains (Split DNS)", "ru": "Исключённые домены (Split DNS)"],
        "split_tunnel_routes": ["en": "Excluded Routes (CIDR)",      "ru": "Исключённые маршруты (CIDR)"],
        "split_tunnel_none":   ["en": "None",                         "ru": "Нет"],
        "record_new_mask":     ["en": "Record New Mask", "ru": "Записать новую маску"],
        "stop_recording":      ["en": "Stop Recording", "ru": "Остановить запись"],
        "record_service_name": ["en": "Mask Service Name", "ru": "Имя сервиса для маски"],
        "recording_starting":  ["en": "Starting recording…", "ru": "Запуск записи…"],
        "recording_active":    ["en": "Recording in progress. Use the service normally.",
                                "ru": "Запись идёт. Используйте сервис как обычно."],
        "recording_stopping":  ["en": "Stopping recording…", "ru": "Останавливаем запись…"],
        "recording_analyzing": ["en": "Analyzing traffic…",  "ru": "Анализ трафика…"],
        "recording_success":   ["en": "Mask recorded successfully", "ru": "Маска успешно записана"],
        "recording_failed":    ["en": "Mask recording failed",      "ru": "Запись маски не удалась"],
        "recording_self_test_failed": ["en": "Mask did not pass verification",
                                       "ru": "Маска не прошла проверку"],
        "recording_result_success_title": ["en": "Last recording: saved",
                                           "ru": "Результат: маска сохранена"],
        "recording_result_failed_title":  ["en": "Last recording: not saved",
                                           "ru": "Результат: маска не сохранена"],
        "dismiss":             ["en": "Dismiss",  "ru": "Скрыть"],
        "upload":              ["en": "Upload",   "ru": "Исходящий"],
        "download":            ["en": "Download", "ru": "Входящий"],
        "duration":            ["en": "Duration", "ru": "Длительность"],
        "version_footer":      ["en": "v\(LocalizationManager.appVersion) · Neural Resonance VPN",
                                "ru": "v\(LocalizationManager.appVersion) · Neural Resonance VPN"],
        "error_invalid_key":   ["en": "Invalid connection key format",
                                "ru": "Неверный формат ключа подключения"],
        "no_profiles":         ["en": "No saved keys. Tap + to add one.",
                                "ru": "Нет ключей. Нажмите + для добавления."],
        "recording_ready":     ["en": "Recording availability is checked by the server when you start",
                                "ru": "Доступ к записи проверяется сервером при запуске"],
        "adaptive_mode":       ["en": "Adaptive Mode",          "ru": "Адаптивный режим"],
        "adaptive_mode_help":  ["en": "Auto-adjusts keepalive and FEC on unstable connections",
                                "ru": "Автоматически адаптирует keepalive и FEC при нестабильном соединении"],
        "adaptive_off":        ["en": "Off",                    "ru": "Выкл"],
        "adaptive_light":      ["en": "Light (6s)",             "ru": "Лёгкий (6с)"],
        "adaptive_aggressive": ["en": "Aggressive (4s)",        "ru": "Агрессивный (4с)"],
        "adaptive_satellite":  ["en": "Satellite (15s)",        "ru": "Спутник (15с)"],
        "diagnostics":         ["en": "Diagnostics",            "ru": "Диагностика"],
        "run_benchmark":       ["en": "Run Benchmark",          "ru": "Запустить тест"],
        "bench_running":       ["en": "Running benchmark…",     "ru": "Тест запущен…"],
        "bench_idle":          ["en": "Tap to measure latency and connection quality.",
                                "ru": "Нажмите для измерения задержки и качества соединения."],
        "mtls_cert_hint":      ["en": "mTLS cert (base64, leave empty to disable)",
                                "ru": "mTLS сертификат (base64, оставьте пустым для отключения)"],
        "kill_switch":         ["en": "Kill Switch (block traffic if VPN drops)",
                                "ru": "Kill Switch (блок трафика при разрыве VPN)"],
    ]

    init() {
        let systemLang: String?
        if #available(iOS 16, *) {
            systemLang = Locale.current.language.languageCode?.identifier
        } else {
            systemLang = Locale.current.languageCode
        }
        language = UserDefaults.standard.string(forKey: "app_language") ?? systemLang ?? "en"
        if language != "en" && language != "ru" { language = "en" }
    }

    func t(_ key: String) -> String {
        guard let dict = strings[key] else { return key }
        return dict[language] ?? dict["en"] ?? key
    }

    func toggleLanguage() {
        language = language == "en" ? "ru" : "en"
    }
}
