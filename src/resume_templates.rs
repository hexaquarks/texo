use crate::editor::STARTER_DOCUMENT;

pub struct ResumeTemplate {
    pub name: &'static str,
    pub description: &'static str,
    pub source: &'static str,
    pub thumbnail: &'static [u8],
}

// Thumbnails show the first page rendered by Tectonic and Poppler from each bundled source.
pub static TEMPLATES: &[ResumeTemplate] = &[
    ResumeTemplate {
        name: "Blank",
        description: "Start with the default LaTeX document.",
        source: STARTER_DOCUMENT,
        thumbnail: include_bytes!("../assets/resume-templates/blank.png"),
    },
    ResumeTemplate {
        name: "Jake's Resume",
        description: "Compact engineering resume by Jake Gutierrez (MIT).",
        source: include_str!("../assets/resume-templates/jake.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/jake.png"),
    },
    ResumeTemplate {
        name: "Rezume",
        description: "Single-column resume by Nanu Panchamurthy (MIT).",
        source: include_str!("../assets/resume-templates/rezume.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/rezume.png"),
    },
    ResumeTemplate {
        name: "AustereCV",
        description: "Compact resume with a summary by Shubham Mazumder (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/austere.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/austere.png"),
    },
    ResumeTemplate {
        name: "Software Engineering",
        description: "Single-column resume with blue headings by Leslie Cheng (MIT).",
        source: include_str!("../assets/resume-templates/cheng.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/cheng.png"),
    },
    ResumeTemplate {
        name: "Modern Simple CV",
        description: "Experience-first resume by Yousef (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/marketing.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/marketing.png"),
    },
    ResumeTemplate {
        name: "Minimal CV Template",
        description: "Single-column CV by Matheus Hostert (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/minimal.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/minimal.png"),
    },
    ResumeTemplate {
        name: "Libre CV",
        description: "Two-column resume by Samuel Boïté (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/libre.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/libre.png"),
    },
    ResumeTemplate {
        name: "Cole's Resume",
        description: "Education-first resume by Cole Miller (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/coles.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/coles.png"),
    },
    ResumeTemplate {
        name: "Academic CV by Diego",
        description: "Multi-page academic CV by Diego Damasceno (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/diego.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/diego.png"),
    },
    ResumeTemplate {
        name: "Resume by Gabriel",
        description: "Research and skills resume by Gabriel G. Carvalho (CC BY 4.0).",
        source: include_str!("../assets/resume-templates/gabriel.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/gabriel.png"),
    },
    ResumeTemplate {
        name: "Minimal LaTeX CV",
        description: "CV with a skills chart by Jan Vorisek (MIT).",
        source: include_str!("../assets/resume-templates/jan.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/jan.png"),
    },
    ResumeTemplate {
        name: "LaTeX Resume Template",
        description: "Software resume by Joshua Soberg (MIT).",
        source: include_str!("../assets/resume-templates/josh.tex"),
        thumbnail: include_bytes!("../assets/resume-templates/josh.png"),
    },
];
