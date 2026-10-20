//! Filtro de segredos: roda sobre todo texto vindo de alerta antes de ele
//! entrar no contexto enviado à IA. Troca só o valor por `[removido]` e mantém o
//! nome, para a IA ainda entender o que aconteceu.

use regex::{Captures, Regex};
use std::sync::OnceLock;

const REMOVIDO: &str = "[removido]";

/// Palavras que, no nome de uma variável, flag ou chave, indicam segredo.
const PALAVRAS: &str = "token|pass(?:word|wd)?|secret|key|auth|credential|cookie|session";

/// Valor: o marcador já posto (idempotência), entre aspas, aspas sem fechar ou até o próximo espaço/`&`.
const VALOR: &str = r#"\[removido\]|"[^"]*"|'[^']*'|["'][^\s&]*|[^\s&"']+"#;

struct Regras {
    /// `NOME_EM_MAIUSCULO=valor` (qualquer nome em caixa-alta).
    env: Regex,
    /// Cabeçalhos com credencial: vale até o fim do argumento (aspas ou linha).
    cabecalhos: Regex,
    /// `Bearer x` solto.
    bearer: Regex,
    /// `--nome-com-segredo=x` ou `--nome-com-segredo x`.
    flags: Regex,
    /// `-u usuario:senha` / `--user=usuario:senha` (só quando o valor tem `:`).
    usuario: Regex,
    /// `nome=valor`, `nome: valor`, `'nome': 'valor'` com nome suspeito.
    generico: Regex,
    /// Formatos conhecidos de chave e JWT.
    chaves: Regex,
    /// `://qualquer-coisa@` no começo de uma URL (usuário, senha ou token).
    url: Regex,
}

fn regras() -> &'static Regras {
    static R: OnceLock<Regras> = OnceLock::new();
    R.get_or_init(|| Regras {
        env: Regex::new(r#"(^|[^A-Za-z0-9_])([A-Z][A-Z0-9_]{2,}=)("[^"]*"|'[^']*'|\S+)"#).unwrap(),
        cabecalhos: Regex::new(
            r#"(?i)((?:proxy-)?authorization|x-api-key|x-auth-token|cookie|set-cookie)(["']?\s*:\s*)(\[removido\]|"[^"]*"|'[^']*'|[^"'\n\r]*)"#,
        )
        .unwrap(),
        bearer: Regex::new(r"(?i)\b(bearer\s+)[A-Za-z0-9._~+/=-]+").unwrap(),
        flags: Regex::new(&format!(
            r#"(?i)(--[A-Za-z0-9-]*(?:{PALAVRAS})[A-Za-z0-9-]*)(=\s*|\s+)({VALOR})"#
        ))
        .unwrap(),
        usuario: Regex::new(r#"(^|\s)(-u|--user)(=|\s+)(["']?[^\s:"']+:[^\s"']*["']?)"#).unwrap(),
        generico: Regex::new(&format!(
            r#"(?i)([A-Za-z0-9_./:-]*(?:{PALAVRAS})[A-Za-z0-9_-]*)(["']?\s*[=:]\s*)({VALOR})"#
        ))
        .unwrap(),
        chaves: Regex::new(
            r"AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|npm_[A-Za-z0-9]{30,}|sk-[A-Za-z0-9_-]{20,}|xox[abpr]-[A-Za-z0-9-]+|eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        )
        .unwrap(),
        url: Regex::new(r#"://[^/\s?#"']*@"#).unwrap(),
    })
}

/// O nome tem alguma palavra de segredo? `keychain` (caminho do Keychain) e
/// `author` (autor) contêm `key`/`auth` por acaso e não contam; `authorization` conta.
fn suspeito(nome: &str) -> bool {
    static P: OnceLock<Regex> = OnceLock::new();
    let p = P.get_or_init(|| Regex::new(&format!("(?i){PALAVRAS}")).unwrap());
    let n = nome
        .to_lowercase()
        .replace("authorization", "auth")
        .replace("keychain", "")
        .replace("author", "");
    p.is_match(&n)
}

/// Troca por `[removido]` o valor de qualquer segredo reconhecível no texto.
pub fn redact(s: &str) -> String {
    let r = regras();
    // Primeiro NOME=valor (valores entre aspas), antes que um cabeçalho sem aspas
    // pare na aspa seguinte e deixe o resto da linha passar.
    let s = r
        .env
        .replace_all(s, |c: &Captures| format!("{}{}{REMOVIDO}", &c[1], &c[2]));
    let s = r
        .cabecalhos
        .replace_all(&s, |c: &Captures| format!("{}{}{REMOVIDO}", &c[1], &c[2]));
    let s = r
        .bearer
        .replace_all(&s, |c: &Captures| format!("{}{REMOVIDO}", &c[1]));
    let s = r.flags.replace_all(&s, |c: &Captures| {
        // `--auth --verbose`: o que vem depois é outra flag, não o valor.
        if !suspeito(&c[1]) || (c[2].trim().is_empty() && c[3].starts_with("--")) {
            c[0].to_string()
        } else {
            format!("{}{}{REMOVIDO}", &c[1], &c[2])
        }
    });
    let s = r.usuario.replace_all(&s, |c: &Captures| {
        format!("{}{}{}{REMOVIDO}", &c[1], &c[2], &c[3])
    });
    let s = r.generico.replace_all(&s, |c: &Captures| {
        if suspeito(&c[1]) {
            format!("{}{}{REMOVIDO}", &c[1], &c[2])
        } else {
            c[0].to_string()
        }
    });
    let s = r.chaves.replace_all(&s, REMOVIDO);
    let s = r.url.replace_all(&s, format!("://{REMOVIDO}@"));
    s.into_owned()
}

/// Redige uma lista de argumentos de processo um a um. Um argumento solto logo
/// depois de uma flag secreta (`--password`, `"correct horse battery"`) ou de `-u`
/// com `:` some inteiro, mesmo tendo espaços.
pub fn redact_args(args: &[String]) -> String {
    // `--nome=valor com espaços` num único elemento do argv.
    static IGUAL: OnceLock<Regex> = OnceLock::new();
    let igual = IGUAL.get_or_init(|| Regex::new(r"(?s)^(-{0,2}[A-Za-z0-9_.-]+)=(.*)$").unwrap());
    let mut saida: Vec<String> = Vec::with_capacity(args.len());
    let mut anterior: Option<&str> = None;
    for a in args {
        let segredo = match anterior {
            Some(p) if p.starts_with("--") && !p.contains('=') && suspeito(p) => {
                !a.starts_with("--")
            }
            Some("-u") | Some("--user") => a.contains(':'),
            _ => false,
        };
        let com_igual = igual.captures(a).filter(|c| {
            let nome = &c[1];
            let maiusculo = nome.len() >= 3
                && nome.starts_with(|ch: char| ch.is_ascii_uppercase())
                && nome
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
            let usuario = nome == "--user" && c[2].contains(':');
            suspeito(nome) || maiusculo || usuario
        });
        saida.push(if segredo {
            REMOVIDO.to_string()
        } else if let Some(c) = com_igual {
            format!("{}={REMOVIDO}", &c[1])
        } else {
            redact(a)
        });
        anterior = Some(a);
    }
    saida.join(" ")
}

/// Linha de comando já juntada num texto só (`args.join(" ")`, como o daemon põe
/// no detalhe e no alvo de `third_party_shell`): sem o argv, não dá para saber
/// onde o valor de uma flag secreta termina. Depois de `--nome-secreto`,
/// `--nome-secreto=` ou de `-u`/`--user` com `:`, some tudo até o próximo token
/// que comece com `-` (ou até o fim). Apaga a mais de propósito; o resto passa
/// por `redact`.
pub fn redact_cmdline(s: &str) -> String {
    static NOME: OnceLock<Regex> = OnceLock::new();
    let nome_ok = NOME.get_or_init(|| Regex::new(r"^-{0,2}[A-Za-z0-9_.-]+$").unwrap());
    let toks: Vec<&str> = s.split(' ').collect();
    // Primeiro token depois de `j` que começa com `-` (onde o valor acaba).
    let fim_do_valor = |mut j: usize| {
        while j < toks.len() && !toks[j].starts_with('-') {
            j += 1;
        }
        j
    };
    let mut saida: Vec<String> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        let proximo_e_valor = toks.get(i + 1).is_some_and(|p| !p.starts_with('-'));
        if let Some((nome, valor)) = t.split_once('=') {
            let segredo = nome_ok.is_match(nome) && suspeito(nome);
            let usuario = nome == "--user" && valor.contains(':');
            if segredo || usuario {
                saida.push(format!("{nome}={REMOVIDO}"));
                i = fim_do_valor(i + 1);
                continue;
            }
        } else {
            let flag = t.starts_with("--") && nome_ok.is_match(t) && suspeito(t);
            let usuario =
                (t == "-u" || t == "--user") && toks.get(i + 1).is_some_and(|p| p.contains(':'));
            if (flag && proximo_e_valor) || usuario {
                saida.push(t.to_string());
                saida.push(REMOVIDO.to_string());
                i = fim_do_valor(i + 1);
                continue;
            }
        }
        saida.push(t.to_string());
        i += 1;
    }
    redact(&saida.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_header() {
        let o = redact(r#"curl -H "Authorization: Bearer abc.def" https://x.com"#);
        assert!(!o.contains("abc.def"));
        assert!(o.contains("Authorization: [removido]"));
        assert!(o.contains("https://x.com"));
    }

    #[test]
    fn aws_key() {
        assert!(!redact("chave AKIAABCDEFGHIJKLMNOP aqui").contains("AKIAABCDEFGHIJKLMNOP"));
    }

    #[test]
    fn github_and_openai() {
        let gh = "ghp_abcdefghijklmnopqrstuvwxyz0123";
        let sk = "sk-abcdefghijklmnopqrstuvwx_yz-01";
        let o = redact(&format!("a {gh} b {sk} c"));
        assert!(!o.contains(gh) && !o.contains(sk));
        assert_eq!(o, "a [removido] b [removido] c");
    }

    #[test]
    fn url_credentials() {
        assert_eq!(
            redact("https://natan:t0k3n@github.com/x"),
            "https://[removido]@github.com/x"
        );
    }

    #[test]
    fn env_assignment() {
        assert_eq!(
            redact("AWS_SECRET_ACCESS_KEY=zzz node app.js"),
            "AWS_SECRET_ACCESS_KEY=[removido] node app.js"
        );
    }

    #[test]
    fn flags() {
        let o = redact("cli --token=abc --password abc2 --secret 'a b' --api-key=k9");
        for v in ["abc", "abc2", "a b", "k9"] {
            assert!(
                !o.contains(&format!("={v}")) && !o.contains(&format!(" {v}")),
                "{o}"
            );
        }
        assert!(o.contains("--token=[removido]"));
        assert!(o.contains("--password [removido]"));
    }

    #[test]
    fn keeps_normal_text() {
        let t = "node /Users/dev/x/server.js --port 3000 -p 80 PATH ok";
        assert_eq!(redact(t), t);
    }

    #[test]
    fn extra_shapes() {
        let o = redact(
            r#"{"password": "hunter2"} xoxb-123-abc Cookie: sid=abc; x=1 MY_VAR="a b c" end"#,
        );
        for v in ["hunter2", "xoxb-123-abc", "sid=abc", "a b c"] {
            assert!(!o.contains(v), "{o}");
        }
    }

    /// Nenhum dos valores pode sobrar na saída, e redigir de novo não muda nada.
    fn sem_vazar(entrada: &str, segredos: &[&str]) {
        let o = redact(entrada);
        for v in segredos {
            assert!(!o.contains(v), "vazou {v:?} em {o:?}");
        }
        assert_eq!(redact(&o), o, "não idempotente: {o:?}");
    }

    #[test]
    fn lowercase_key_value() {
        sem_vazar("mysql --user=root password=hunter2", &["hunter2"]);
        sem_vazar("Password=x1y2z3", &["x1y2z3"]);
        sem_vazar("secret=abcdef", &["abcdef"]);
        sem_vazar(
            "npm config set //registry.npmjs.org/:_authToken=npm_AbCdEf0123456789AbCdEf0123456789abcd",
            &["AbCdEf0123456789"],
        );
        assert!(redact("mysql --user=root password=hunter2").contains("--user=root"));
    }

    #[test]
    fn query_string() {
        let o = redact("https://api.x.com/v1?token=abc123&api_key=def456");
        assert!(!o.contains("abc123") && !o.contains("def456"), "{o}");
        assert!(o.contains("api.x.com/v1?token="));
        assert_eq!(redact(&o), o);
    }

    #[test]
    fn jwt() {
        sem_vazar(
            "node x.js eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl",
            &["eyJhbGci", "c2lnbmF0dXJl"],
        );
    }

    #[test]
    fn curl_user() {
        sem_vazar("curl -u natan:hunter2 https://api.x.com", &["hunter2"]);
        sem_vazar("curl --user natan:hunter2 https://api.x.com", &["hunter2"]);
        assert_eq!(redact("ls -u root"), "ls -u root");
    }

    #[test]
    fn flag_names_outside_list() {
        for (e, v) in [
            ("--auth abc", "abc"),
            ("--key k", " k"),
            ("--pass p", " p"),
            ("--db-password=hunter2", "hunter2"),
            ("--client-secret abc", "abc"),
            ("--secret-key=abc", "abc"),
            ("--aws-secret-access-key X", " X"),
            ("--private-key=/k/ab12", "ab12"),
        ] {
            let o = redact(e);
            assert!(!o.contains(v), "{e} -> {o}");
            assert_eq!(redact(&o), o);
        }
        // Outra flag logo depois não é valor.
        assert_eq!(redact("cmd --auth --verbose"), "cmd --auth --verbose");
    }

    #[test]
    fn quoted_and_code() {
        sem_vazar(
            "node -e \"fetch(u,{headers:{Authorization:'Basic dXNlcjpwYXNz'}})\"",
            &["dXNlcjpwYXNz"],
        );
        sem_vazar(r#"Authorization: "Basic dXNlcjpwYXNz""#, &["dXNlcjpwYXNz"]);
        sem_vazar("{'password': 'hunter2'}", &["hunter2"]);
        sem_vazar("password: hunter2", &["hunter2"]);
        sem_vazar("token: 'x9y8z7'", &["x9y8z7"]);
    }

    #[test]
    fn token_only_url() {
        assert_eq!(
            redact("git clone https://glpat-AbCdEfGhIjKlMnOpQrSt@gitlab.com/x.git"),
            "git clone https://[removido]@gitlab.com/x.git"
        );
    }

    #[test]
    fn url_scope_and_at_in_password() {
        let u = "https://registry.npmjs.org:443/@scope/pkg";
        assert_eq!(redact(u), u);
        let o = redact("https://u:p@ss@host/x");
        assert_eq!(o, "https://[removido]@host/x");
    }

    #[test]
    fn args_one_by_one() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = redact_args(&a(&[
            "tool",
            "--password",
            "correct horse battery",
            "--port",
            "3000",
        ]));
        assert_eq!(o, "tool --password [removido] --port 3000");
        let o = redact_args(&a(&["curl", "-u", "natan:hunter2"]));
        assert!(!o.contains("hunter2"));
    }

    #[test]
    fn args_with_equals_and_spaces() {
        let um = |x: &str| redact_args(&[x.to_string()]);
        assert_eq!(
            um("--password=correct horse battery"),
            "--password=[removido]"
        );
        assert_eq!(um("password=correct horse battery"), "password=[removido]");
        assert_eq!(um("PASSWORD=correct horse battery"), "PASSWORD=[removido]");
        assert_eq!(um("--user=natan:hunter 2"), "--user=[removido]");
        assert_eq!(um("--port=3000"), "--port=3000");
    }

    #[test]
    fn keychain_and_author_stay() {
        let t = "--keychain /Users/dev/Library/Keychains/login.keychain-db";
        assert_eq!(redact(t), t);
        assert_eq!(redact("author=natan"), "author=natan");
        assert_eq!(
            redact("author=natan --token=abc"),
            "author=natan --token=[removido]"
        );
        assert_eq!(
            redact("Authorization: Bearer x1"),
            "Authorization: [removido]"
        );
        let a = vec!["--keychain".to_string(), "/x/login.keychain-db".to_string()];
        assert_eq!(redact_args(&a), "--keychain /x/login.keychain-db");
    }

    #[test]
    fn bearer_standalone() {
        assert_eq!(redact("Bearer abc123"), "Bearer [removido]");
        let o = redact("curl -H 'X-Foo: Bearer abc123'");
        assert!(!o.contains("abc123"), "{o}");
        assert_eq!(redact(&o), o);
        let a = ["curl", "-H", "Bearer abc123"].map(String::from);
        assert!(!redact_args(&a).contains("abc123"));
    }

    #[test]
    fn cmdline_joined_with_spaces() {
        for (e, segredos) in [
            (
                "mysql --password=correct horse battery",
                &["correct", "horse", "battery"][..],
            ),
            ("x --token a b c --port 3000", &["a b c", " b ", " c "][..]),
            ("curl -u natan:hunter 2 -v", &["hunter", " 2 "][..]),
            ("curl --user=natan:hunter 2", &["hunter", " 2"][..]),
            ("tool password=um dois", &["um", "dois"][..]),
        ] {
            let o = redact_cmdline(e);
            for v in segredos {
                assert!(!o.contains(v), "{e} -> {o} (vazou {v:?})");
            }
            assert_eq!(redact_cmdline(&o), o, "não idempotente: {o}");
        }
        assert_eq!(
            redact_cmdline("x --token a b c --port 3000"),
            "x --token [removido] --port 3000"
        );
        // Linha comum passa igual.
        let t = "node server.js --port 3000 --keychain /k/login.keychain-db";
        assert_eq!(redact_cmdline(t), t);
        assert_eq!(
            redact_cmdline("cmd --auth --verbose"),
            "cmd --auth --verbose"
        );
    }

    #[test]
    fn cabecalhos_sem_aspas() {
        for (e, v) in [
            ("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"),
            ("Cookie: a=1; b=2", "a=1"),
            ("Cookie: a=1; b=2", "b=2"),
        ] {
            let o = redact(e);
            assert!(!o.contains(v), "{e} -> {o}");
            assert_eq!(redact(&o), o);
        }
        assert_eq!(
            redact("Authorization: Basic dXNlcjpwYXNz"),
            "Authorization: [removido]"
        );
        assert_eq!(redact("Cookie: a=1; b=2"), "Cookie: [removido]");
    }
}
