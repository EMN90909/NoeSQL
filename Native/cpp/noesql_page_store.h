#ifndef NOESQL_PAGE_STORE_H
#define NOESQL_PAGE_STORE_H
#include <cstdint>
#include <array>
#include <map>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace noesql {
constexpr size_t kPageSize=4096;
struct Limits {
    uint64_t max_file_bytes=64ull*1024*1024;
    size_t max_key_bytes=128;
    size_t max_value_bytes=1024;
    size_t max_mutations=10000;
    size_t max_scan_rows=100000;
    uint64_t max_transaction_ms=30000;
};
struct Mutation {
    enum class Kind { Put, Delete };
    Kind kind;
    std::string key;
    std::string value;
};
class Session {
    friend class Store;
    std::string name_;
    std::array<uint8_t,32> verifier_{};
    Session(std::string name,std::array<uint8_t,32> verifier):name_(std::move(name)),verifier_(verifier){}
public:
    Session(const Session&)=default;
};
class Store {
public:
    explicit Store(std::string path, Limits limits={});
    // Init requires an explicit administrator password, never a built-in default.
    void initialize(const std::string& admin_password);
    void add_user(const std::string& admin_password, const std::string& name,
                  const std::string& password, bool can_write);
    void remove_user(const std::string& admin_password,const std::string& name);
    Session login(const std::string& name,const std::string& password) const;
    std::optional<std::string> get(const Session& session,const std::string& key) const;
    std::vector<std::pair<std::string,std::string>> scan(const Session& session,const std::string& prefix="") const;
    void transact(const Session& session,const std::vector<Mutation>& mutations) const;
    std::optional<std::string> get(const std::string& name, const std::string& password,
                                   const std::string& key) const;
    std::vector<std::pair<std::string,std::string>> scan(const std::string& name,
                               const std::string& password, const std::string& prefix="") const;
    void transact(const std::string& name, const std::string& password,
                  const std::vector<Mutation>& mutations) const;
    // Reads verify both page checksums and tree structure. No trust in caller data.
    uint64_t verify() const;
private:
    std::string path_;
    Limits limits_;
};
}
#endif
