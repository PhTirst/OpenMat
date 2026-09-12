//! OpenMat-authored minimal XML fixtures, assembled in memory. These are not
//! extracted MATLAB files; real local MATLAB packages are tested separately.
use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

pub const MAIN: &str = "model/document.xml";
pub const SYSTEM: &str = "graphs/top.xml";
pub const CONFIG: &str = "settings/active.xml";
pub const RELEASE: &str = "metadata/release.xml";
pub const ROOT_RELS: &str = "_rels/.rels";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

#[derive(Clone)]
pub struct Fixture {
    pub parts: BTreeMap<String, String>,
}

impl Fixture {
    pub fn feedback() -> Self {
        let entries = [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/></Types>"#,
            ),
            (
                ROOT_RELS,
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="model" Type="http://schemas.mathworks.com/simulink/2010/relationships/blockDiagram" Target="model/document.xml"/>
<Relationship Id="config" Type="http://schemas.mathworks.com/simulink/2014/relationships/configSetInfo" Target="settings/index.xml"/>
<Relationship Id="release" Type="http://schemas.mathworks.com/package/2012/relationships/coreProperties" Target="metadata/release.xml"/>
</Relationships>"#,
            ),
            (
                MAIN,
                r#"<ModelInformation Version="1.0"><Model><P Name="Description">OpenMat miniature model</P><SimulationSettings><P Name="SimulationMode">normal</P></SimulationSettings><System Ref="top"/></Model></ModelInformation>"#,
            ),
            (
                "model/_rels/document.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="top" Type="http://schemas.mathworks.com/simulink/2010/relationships/system" Target="../graphs/top.xml"/></Relationships>"#,
            ),
            (
                "settings/index.xml",
                r#"<ConfigSetInfo><ConfigSet PartName="/settings/active.xml" Active="true">OpenMat</ConfigSet></ConfigSetInfo>"#,
            ),
            (
                "settings/_rels/index.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="active" Type="http://schemas.mathworks.com/simulink/2014/relationships/configSet" Target="active.xml"/></Relationships>"#,
            ),
            (
                CONFIG,
                r#"<ConfigSet><Object ClassName="Simulink.ConfigSet"><P Name="Description">first</P><Array>
<Object ClassName="Simulink.SolverCC"><P Name="Description">second</P><P Name="SolverName">ode4</P><P Name="FixedStep">0.05</P><P Name="StartTime">0</P><P Name="StopTime">1</P><P Name="SampleTimeProperty" Class="double">[]</P></Object>
<Object ClassName="Simulink.DataIOCC"><P Name="LoadExternalInput">off</P><P Name="LoadInitialState">off</P></Object>
</Array></Object></ConfigSet>"#,
            ),
            (
                RELEASE,
                r#"<coreProperties xmlns="http://schemas.mathworks.com/package/2012/coreProperties"><matlabRelease>R2022b</matlabRelease></coreProperties>"#,
            ),
            (
                SYSTEM,
                r#"<System><P Name="Location">[0 0 640 480]</P>
<Block BlockType="Constant" Name="输入 &amp; source" SID="1"><P Name="Value">1</P></Block>
<Block BlockType="Sum" Name="sum" SID="2"><P Name="Inputs">+-</P><PortCounts in="2" out="1"/></Block>
<Block BlockType="Integrator" Name="state" SID="3"><P Name="Position">[190 60 220 90]</P><PortProperties><Port Type="out" Index="1"><P Name="Name">observed</P></Port></PortProperties></Block>
<Block BlockType="Gain" Name="feedback" SID="4"><P Name="Gain">1</P></Block>
<Block BlockType="Scope" Name="result" SID="5"><P Name="NumInputPorts">1</P></Block>
<Line><P Name="Src">1#out:1</P><P Name="Dst">2#in:1</P></Line>
<Line><P Name="Src">2#out:1</P><P Name="Dst">3#in:1</P></Line>
<Line><P Name="Src">3#out:1</P><P Name="Name">observed</P><P Name="Points">[0,0]</P><Branch><P Name="Dst">4#in:1</P></Branch><Branch><P Name="Dst">5#in:1</P></Branch></Line>
<Line><P Name="Src">4#out:1</P><P Name="Dst">2#in:2</P></Line>
</System>"#,
            ),
        ];
        Self {
            parts: entries
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    pub fn edit(&mut self, part: &str, from: &str, to: &str) {
        let text = self.parts.get_mut(part).unwrap();
        assert!(text.contains(from), "fixture edit missing {from}");
        *text = text.replace(from, to);
    }

    pub fn package(&self) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, value) in &self.parts {
            writer
                .start_file(
                    name,
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(value.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    pub fn add_relationship(&mut self, id: &str, kind: &str, target: &str, external: bool) {
        self.edit(ROOT_RELS, "</Relationships>", &format!(r#"<Relationship Id="{id}" Type="{kind}" Target="{target}" TargetMode="{}"/></Relationships>"#, if external { "External" } else { "Internal" }));
    }

    pub fn add_system_relationship(&mut self, source: &str, id: &str, target: &str) {
        self.parts.insert(source.into(), format!(r#"<Relationships xmlns="{REL_NS}"><Relationship Id="{id}" Type="http://schemas.mathworks.com/simulink/2010/relationships/system" Target="{target}"/></Relationships>"#));
    }
}
