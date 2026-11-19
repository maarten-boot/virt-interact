//! The read-only vSphere API calls (SPEC-021): the ONLY requests this app
//! can send. A [`Request`] can be made only by the constructors below, one
//! per allowed method, and the inventory query is fixed here; nothing else
//! can be built or posted ([`crate::http::SoapHttp::post`] takes a
//! `Request`). Covered by TC-130.
//!
//! Allowed: `RetrieveServiceContent`, `SessionManager.Login`,
//! `SessionManager.Logout`, `PropertyCollector.RetrievePropertiesEx`,
//! `PropertyCollector.ContinueRetrievePropertiesEx`.

use std::fmt;

use vi_ssh::Secret;

pub const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
/// The SOAPAction of every request (the API version the app speaks; servers
/// from 6.7 on answer it).
pub const SOAP_ACTION: &str = "urn:vim25/6.7";

/// The allowed methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    RetrieveServiceContent,
    Login,
    Logout,
    RetrievePropertiesEx,
    ContinueRetrievePropertiesEx,
}

impl Method {
    pub const ALL: [Method; 5] = [
        Method::RetrieveServiceContent,
        Method::Login,
        Method::Logout,
        Method::RetrievePropertiesEx,
        Method::ContinueRetrievePropertiesEx,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Method::RetrieveServiceContent => "RetrieveServiceContent",
            Method::Login => "Login",
            Method::Logout => "Logout",
            Method::RetrievePropertiesEx => "RetrievePropertiesEx",
            Method::ContinueRetrievePropertiesEx => "ContinueRetrievePropertiesEx",
        }
    }
}

/// A managed object reference, e.g. `Folder` / `group-d1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MoRef {
    pub kind: String,
    pub value: String,
}

impl MoRef {
    pub fn new(kind: &str, value: &str) -> MoRef {
        MoRef {
            kind: kind.into(),
            value: value.into(),
        }
    }

    fn this(&self) -> String {
        format!(
            "<_this type=\"{}\">{}</_this>",
            escape(&self.kind),
            escape(&self.value)
        )
    }
}

/// One request envelope. Its text is wiped when dropped (the login request
/// holds the password).
pub struct Request {
    method: Method,
    body: Vec<u8>,
}

impl Request {
    fn new(method: Method, inner: &str) -> Request {
        let mut body = Vec::with_capacity(inner.len() + 300);
        body.extend_from_slice(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
              <soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\" \
              xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><soapenv:Body>",
        );
        body.extend_from_slice(format!("<{} xmlns=\"urn:vim25\">", method.name()).as_bytes());
        body.extend_from_slice(inner.as_bytes());
        body.extend_from_slice(format!("</{}>", method.name()).as_bytes());
        body.extend_from_slice(b"</soapenv:Body></soapenv:Envelope>");
        Request { method, body }
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// `ServiceInstance.RetrieveServiceContent`.
    pub fn service_content() -> Request {
        Request::new(
            Method::RetrieveServiceContent,
            &MoRef::new("ServiceInstance", "ServiceInstance").this(),
        )
    }

    /// `SessionManager.Login`.
    pub fn login(session_manager: &MoRef, user: &str, password: &Secret) -> Request {
        let mut inner = session_manager.this().into_bytes();
        inner.extend_from_slice(
            format!("<userName>{}</userName><password>", escape(user)).as_bytes(),
        );
        let pw = escape(password.expose());
        inner.extend_from_slice(pw.as_bytes());
        wipe_string(pw);
        inner.extend_from_slice(b"</password>");
        let text = String::from_utf8(inner).unwrap_or_default();
        let r = Request::new(Method::Login, &text);
        wipe_string(text);
        r
    }

    /// `SessionManager.Logout`.
    pub fn logout(session_manager: &MoRef) -> Request {
        Request::new(Method::Logout, &session_manager.this())
    }

    /// The inventory query (`RetrievePropertiesEx`): VMs and the objects
    /// needed to place and describe them, walked from the root folder with
    /// traversal specs (no server-side view objects are created).
    pub fn inventory(property_collector: &MoRef, root_folder: &MoRef, max_objects: u32) -> Request {
        let mut s = property_collector.this();
        s.push_str("<specSet>");
        for (kind, paths) in INVENTORY_PROPERTIES {
            s.push_str(&format!("<propSet><type>{kind}</type>"));
            for p in *paths {
                s.push_str(&format!("<pathSet>{p}</pathSet>"));
            }
            s.push_str("</propSet>");
        }
        s.push_str(&format!(
            "<objectSet><obj type=\"{}\">{}</obj><skip>false</skip>",
            escape(&root_folder.kind),
            escape(&root_folder.value)
        ));
        for (name, kind, path, next) in TRAVERSAL {
            s.push_str(&format!(
                "<selectSet xsi:type=\"TraversalSpec\"><name>{name}</name><type>{kind}</type>\
                 <path>{path}</path><skip>false</skip>"
            ));
            for n in *next {
                s.push_str(&format!("<selectSet><name>{n}</name></selectSet>"));
            }
            s.push_str("</selectSet>");
        }
        s.push_str("</objectSet></specSet>");
        s.push_str(&format!(
            "<options><maxObjects>{}</maxObjects></options>",
            max_objects.max(1)
        ));
        Request::new(Method::RetrievePropertiesEx, &s)
    }

    /// The next page of the inventory query.
    pub fn continue_inventory(property_collector: &MoRef, token: &str) -> Request {
        Request::new(
            Method::ContinueRetrievePropertiesEx,
            &format!(
                "{}<token>{}</token>",
                property_collector.this(),
                escape(token)
            ),
        )
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        for b in self.body.iter_mut() {
            *b = std::hint::black_box(0);
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the body: it may hold the password.
        write!(f, "Request({})", self.method.name())
    }
}

fn wipe_string(s: String) {
    let mut v = s.into_bytes();
    for b in v.iter_mut() {
        *b = std::hint::black_box(0);
    }
}

/// The properties read per object type (SPEC-021).
pub const INVENTORY_PROPERTIES: &[(&str, &[&str])] = &[
    (
        "VirtualMachine",
        &[
            "name",
            "parent",
            "parentVApp",
            "config.uuid",
            "config.template",
            "config.guestFullName",
            "config.guestId",
            "config.firmware",
            "config.version",
            "config.annotation",
            "config.hardware.numCPU",
            "config.hardware.numCoresPerSocket",
            "config.hardware.memoryMB",
            "config.hardware.device",
            "runtime.powerState",
            "runtime.host",
            "guest.toolsRunningStatus",
            "guest.toolsVersionStatus2",
        ],
    ),
    ("HostSystem", &["name", "parent"]),
    ("ComputeResource", &["name", "parent"]),
    ("Datacenter", &["name", "parent"]),
    ("Folder", &["name", "parent"]),
    ("VirtualApp", &["name", "parent", "parentFolder"]),
    ("Network", &["name"]),
    ("DistributedVirtualPortgroup", &["name", "key"]),
];

/// Traversal from the root folder: (name, type, path, next specs).
pub const TRAVERSAL: &[(&str, &str, &str, &[&str])] = &[
    (
        "folder",
        "Folder",
        "childEntity",
        &[
            "folder",
            "dcVm",
            "dcHost",
            "dcNet",
            "crHost",
            "vappVm",
            "vappChild",
        ],
    ),
    ("dcVm", "Datacenter", "vmFolder", &["folder"]),
    ("dcHost", "Datacenter", "hostFolder", &["folder"]),
    ("dcNet", "Datacenter", "networkFolder", &["folder"]),
    ("crHost", "ComputeResource", "host", &[]),
    ("vappVm", "VirtualApp", "vm", &[]),
    (
        "vappChild",
        "VirtualApp",
        "resourcePool",
        &["vappVm", "vappChild"],
    ),
];

/// XML text escaping for element content and attribute values.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

/// A SOAP fault from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// The fault type from `<detail>`, e.g. `InvalidLogin`, `NotAuthenticated`.
    pub kind: String,
    pub message: String,
}

/// Parse an answer: the first element inside `<Body>`, or the fault.
pub fn parse_body(xml: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse(xml).map_err(|e| format!("unreadable answer: {e}"))
}

/// The fault in a parsed answer, if it is one.
pub fn fault(doc: &roxmltree::Document<'_>) -> Option<Fault> {
    let f = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Fault")?;
    let text = |name: &str| {
        f.children()
            .find(|n| n.tag_name().name() == name)
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let kind = f
        .children()
        .find(|n| n.tag_name().name() == "detail")
        .and_then(|d| d.children().find(|n| n.is_element()))
        .map(|d| {
            d.attribute((XSI, "type"))
                .map(strip_prefix)
                .unwrap_or_else(|| d.tag_name().name().trim_end_matches("Fault"))
                .to_string()
        })
        .unwrap_or_default();
    Some(Fault {
        kind,
        message: text("faultstring"),
    })
}

/// `xsd:string` → `string`.
pub fn strip_prefix(t: &str) -> &str {
    t.rsplit(':').next().unwrap_or(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: &Request) -> String {
        String::from_utf8(r.bytes().to_vec()).unwrap()
    }

    // TC-130: every request is one of the allowed methods, well-formed, and
    // names exactly its method; the password is escaped and not in Debug.
    #[test]
    fn requests() {
        let sm = MoRef::new("SessionManager", "ha-sessionmgr");
        let pc = MoRef::new("PropertyCollector", "ha-property-collector");
        let root = MoRef::new("Folder", "ha-folder-root");
        let pw = Secret::new("p<&>\"'w");
        let all = [
            Request::service_content(),
            Request::login(&sm, "reader@vsphere.local", &pw),
            Request::logout(&sm),
            Request::inventory(&pc, &root, 500),
            Request::continue_inventory(&pc, "tok&1"),
        ];
        for (r, m) in all.iter().zip(Method::ALL) {
            assert_eq!(r.method(), m);
            let t = text(r);
            let doc = roxmltree::Document::parse(&t).unwrap_or_else(|e| panic!("{m:?}: {e}\n{t}"));
            let body = doc
                .descendants()
                .find(|n| n.tag_name().name() == "Body")
                .unwrap();
            let call: Vec<_> = body.children().filter(|n| n.is_element()).collect();
            assert_eq!(call.len(), 1);
            assert_eq!(call[0].tag_name().name(), m.name());
            assert_eq!(call[0].tag_name().namespace(), Some("urn:vim25"));
            for other in Method::ALL.iter().filter(|o| **o != m) {
                assert!(
                    !t.contains(&format!("<{} ", other.name())),
                    "{m:?} contains {other:?}"
                );
            }
        }
        let login = text(&all[1]);
        assert!(
            login.contains("<password>p&lt;&amp;&gt;&quot;&apos;w</password>"),
            "{login}"
        );
        assert_eq!(format!("{:?}", all[1]), "Request(Login)");
        assert!(text(&all[4]).contains("<token>tok&amp;1</token>"));
        // The inventory spec reads, it never names a method.
        let inv = text(&all[3]);
        assert!(
            inv.contains("<type>VirtualMachine</type>")
                && inv.contains("<maxObjects>500</maxObjects>")
        );
    }

    #[test]
    fn faults() {
        let xml = r#"<?xml version="1.0"?><soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><soapenv:Body><soapenv:Fault><faultcode>ServerFaultCode</faultcode><faultstring>Login failure</faultstring><detail><InvalidLoginFault xmlns="urn:vim25" xsi:type="InvalidLogin"></InvalidLoginFault></detail></soapenv:Fault></soapenv:Body></soapenv:Envelope>"#;
        let doc = parse_body(xml).unwrap();
        assert_eq!(
            fault(&doc),
            Some(Fault {
                kind: "InvalidLogin".into(),
                message: "Login failure".into()
            })
        );
        let ok = parse_body("<a><b/></a>").unwrap();
        assert_eq!(fault(&ok), None);
        assert_eq!(strip_prefix("xsd:string"), "string");
        assert_eq!(escape("a<b"), "a&lt;b");
    }
}
