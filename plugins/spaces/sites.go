package spaces

import (
	"archive/tar"
	"compress/gzip"
	"database/sql"
	"io"
	"os"
	"slices"
	"strings"
	"time"

	_ "modernc.org/sqlite"

	"github.com/drevantonder/the-one-app/sdk"
)

// A login's sites are read from its browser's cookies when it is saved:
// the sites with a cookie that only a signed-in visitor has, and when it
// expires.

type site struct {
	Site  string    `json:"site"`
	Until time.Time `json:"until,omitzero"`
}

type loginSites struct {
	Login  string    `json:"login"`
	Sites  []site    `json:"sites"`
	Others int       `json:"others,omitempty"` // other sites with cookies
	At     time.Time `json:"at"`
}

// signedIn are cookies sites set once someone signs in, by site.
var signedIn = map[string][]string{
	"x.com":         {"auth_token"},
	"google.com":    {"SID", "__Secure-1PSID"},
	"facebook.com":  {"c_user"},
	"messenger.com": {"c_user"},
	"instagram.com": {"sessionid"},
	"github.com":    {"user_session"},
	"linkedin.com":  {"li_at"},
	"reddit.com":    {"reddit_session"},
	"amazon.com":    {"at-main"},
	"youtube.com":   {"LOGIN_INFO"},
	"apple.com":     {"myacinfo"},
	"microsoft.com": {"ESTSAUTHPERSISTENT"},
	"live.com":      {"ESTSAUTHPERSISTENT"},
	"discord.com":   {"__dcfduid"},
	"notion.so":     {"token_v2"},
	"slack.com":     {"d"},
}

// domain is a cookie host's site: its last two labels.
func domain(host string) string {
	parts := strings.Split(strings.TrimPrefix(host, "."), ".")
	if len(parts) > 2 {
		parts = parts[len(parts)-2:]
	}
	return strings.Join(parts, ".")
}

// sitesIn reads the sites of a saved login's archive.
func sitesIn(archive string) ([]site, int, error) {
	f, err := os.Open(archive)
	if err != nil {
		return nil, 0, err
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		return nil, 0, err
	}
	r := tar.NewReader(gz)
	tmp, err := os.CreateTemp("", "one-cookies-*.db")
	if err != nil {
		return nil, 0, err
	}
	defer os.Remove(tmp.Name())
	found := false
	for {
		h, err := r.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, 0, err
		}
		if strings.TrimPrefix(h.Name, "./") == "Default/Cookies" {
			if _, err := io.Copy(tmp, r); err != nil {
				return nil, 0, err
			}
			found = true
			break
		}
	}
	tmp.Close()
	if !found {
		return nil, 0, nil
	}
	return sitesInDB(tmp.Name())
}

// liveSites reads the sites a running Space's browser is signed in to, as
// far as it has written its cookies out (within half a minute).
func liveSites(call *sdk.Call, s space, login string) ([]site, int, error) {
	tmp, err := os.CreateTemp("", "one-cookies-*.db")
	if err != nil {
		return nil, 0, err
	}
	defer os.Remove(tmp.Name())
	err = rt.exec(call.Context, s.Name, nil, tmp, "cat", profiles+login+"/Default/Cookies")
	tmp.Close()
	if err != nil {
		return nil, 0, nil // no cookies yet
	}
	return sitesInDB(tmp.Name())
}

// sitesInDB reads the sites in a browser's cookie database.
func sitesInDB(path string) ([]site, int, error) {
	db, err := sql.Open("sqlite", "file:"+path+"?mode=ro")
	if err != nil {
		return nil, 0, err
	}
	defer db.Close()
	rows, err := db.Query(`SELECT host_key, name, expires_utc FROM cookies`)
	if err != nil {
		return nil, 0, err
	}
	defer rows.Close()
	in := map[string]time.Time{}
	others := map[string]bool{}
	for rows.Next() {
		var host, name string
		var expires int64 // microseconds since 1601
		if err := rows.Scan(&host, &name, &expires); err != nil {
			return nil, 0, err
		}
		d := domain(host)
		others[d] = true
		if slices.Contains(signedIn[d], name) {
			until := time.Time{}
			if expires > 0 {
				until = time.UnixMicro(expires - 11644473600*1e6).UTC()
			}
			if until.After(in[d]) || in[d].IsZero() {
				in[d] = until
			}
		}
	}
	var sites []site
	for d, until := range in {
		sites = append(sites, site{Site: d, Until: until})
		delete(others, d)
	}
	slices.SortFunc(sites, func(a, b site) int { return strings.Compare(a.Site, b.Site) })
	return sites, len(others), rows.Err()
}

// siteNames lists sites for people: "x.com and messenger.com", or none.
func siteNames(sites []site) string {
	var names []string
	for _, s := range sites {
		if s.Until.IsZero() || s.Until.After(time.Now()) {
			names = append(names, s.Site)
		}
	}
	switch len(names) {
	case 0:
		return "no site"
	case 1:
		return names[0]
	}
	return strings.Join(names[:len(names)-1], ", ") + " and " + names[len(names)-1]
}
