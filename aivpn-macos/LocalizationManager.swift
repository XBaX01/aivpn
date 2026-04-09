import Foundation
import Combine

class LocalizationManager: ObservableObject {
    static let shared = LocalizationManager()

    @Published var language: String = "en" {
        didSet {
            UserDefaults.standard.set(language, forKey: "app_language")
        }
    }

    private let strings: [String: [String: String]] = [
        "status_connected": [
            "en": "Connected",
            "ru": "Подключено"
        ],
        "status_disconnected": [
            "en": "Disconnected",
            "ru": "Отключено"
        ],
        "enter_key": [
            "en": "Connection key (aivpn://...)",
            "ru": "Ключ подключения (aivpn://...)"
        ],
        "no_key": [
            "en": "No connection key set",
            "ru": "Ключ подключения не задан"
        ],
        "change": [
            "en": "Change",
            "ru": "Изменить"
        ],
        "full_tunnel": [
            "en": "Full tunnel (route all traffic)",
            "ru": "Полный туннель (весь трафик)"
        ],
        "full_tunnel_help": [
            "en": "Route all system traffic through VPN",
            "ru": "Направить весь системный трафик через VPN"
        ],
        "connect": [
            "en": "Connect",
            "ru": "Подключить"
        ],
        "disconnect": [
            "en": "Disconnect",
            "ru": "Отключить"
        ],
        "connecting": [
            "en": "Connecting...",
            "ru": "Подключение..."
        ],
        "quit": [
            "en": "Quit",
            "ru": "Выход"
        ],
        "helper_ready": [
            "en": "Service ready",
            "ru": "Сервис готов"
        ],
        "helper_missing": [
            "en": "Service unavailable — install AIVPN from the .pkg installer",
            "ru": "Сервис недоступен — установите AIVPN через файл .pkg"
        ],
        "helper_starting": [
            "en": "Checking service...",
            "ru": "Проверка сервиса..."
        ],
        "key_name": [
            "en": "Key Name",
            "ru": "Название ключа"
        ],
        "select_key": [
            "en": "Select Key",
            "ru": "Выбрать ключ"
        ],
        "select_key_prompt": [
            "en": "Select a connection key",
            "ru": "Выберите ключ подключения"
        ],
        "add_key": [
            "en": "Add Key",
            "ru": "Добавить ключ"
        ],
        "done": [
            "en": "Done",
            "ru": "Готово"
        ],
        "edit": [
            "en": "Edit",
            "ru": "Изменить"
        ],
        "delete": [
            "en": "Delete",
            "ru": "Удалить"
        ],
        "duplicate_key": [
            "en": "This key already exists",
            "ru": "Этот ключ уже существует"
        ],
        "delete_key_confirm": [
            "en": "Delete Key?",
            "ru": "Удалить ключ?"
        ],
        "delete_key_message": [
            "en": "Are you sure you want to delete this key?",
            "ru": "Вы уверены что хотите удалить этот ключ?"
        ],
        "cancel": [
            "en": "Cancel",
            "ru": "Отмена"
        ],
        "connection_keys": [
            "en": "Connection Keys",
            "ru": "Ключи подключения"
        ],
        "no_keys_yet": [
            "en": "No keys yet",
            "ru": "Нет ключей"
        ],
        "add_first_key": [
            "en": "Add First Key",
            "ru": "Добавить первый ключ"
        ],
        "no_key_selected": [
            "en": "No key selected",
            "ru": "Ключ не выбран"
        ],
        "save_key": [
            "en": "Save",
            "ru": "Сохранить"
        ],
    ]

    init() {
        language = UserDefaults.standard.string(forKey: "app_language") ?? Locale.current.language.languageCode?.identifier ?? "en"
        if language != "en" && language != "ru" {
            language = "en"
        }
    }

    func t(_ key: String) -> String {
        guard let dict = strings[key] else { return key }
        return dict[language] ?? dict["en"] ?? key
    }

    func toggleLanguage() {
        language = language == "en" ? "ru" : "en"
    }
}
