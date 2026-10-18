# Windows PowerShell 5.1 + .NET Framework System.Drawing, offline only.
# This is NOT a general SVG renderer. See assets/navigation/README.md for the exact subset.
[CmdletBinding()]
param([switch]$ValidateOnly, [switch]$SelfTest)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing,System.Xml -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using System.Xml;

public static class NavigationMasks {
    const string Ns = "http://www.w3.org/2000/svg";
    static readonly int[] Sizes = { 18, 27, 36, 48, 72, 144 };
    static readonly Regex Token = new Regex(@"[MLHVZ]|[+-]?(?:\d+(?:\.\d*)?|\.\d+)");

    static void Require(bool value, string message) {
        if (!value) throw new InvalidDataException(message);
    }
    static float Number(string text) {
        float value;
        Require(Regex.IsMatch(text, @"\A(?:\d+(?:\.\d*)?|\.\d+)\z") &&
            float.TryParse(text, NumberStyles.AllowDecimalPoint, CultureInfo.InvariantCulture, out value),
            "Expected a non-negative SVG number: " + text);
        value = float.Parse(text, CultureInfo.InvariantCulture);
        Require(value <= 24, "SVG coordinates must be within 0..24.");
        return value;
    }
    static void Attributes(XmlElement element, params string[] allowed) {
        foreach (XmlAttribute attribute in element.Attributes) {
            Require(Array.IndexOf(allowed, attribute.Name) >= 0, "Unsupported SVG attribute: " + attribute.Name);
        }
        foreach (string name in allowed) {
            Require(element.HasAttribute(name), "Missing SVG attribute: " + name);
        }
    }
    static GraphicsPath Path(string data) {
        MatchCollection matches = Token.Matches(data);
        Require(matches.Count > 0 && matches.Count <= 1024 &&
            Regex.IsMatch(Token.Replace(data, ""), @"\A[\s,]*\z"), "Unsupported SVG path syntax.");
        var tokens = new List<string>();
        foreach (Match match in matches) tokens.Add(match.Value);
        var path = new GraphicsPath();
        try {
            int index = 0;
            float x = 0, y = 0, startX = 0, startY = 0;
            bool open = false;
            while (index < tokens.Count) {
                string command = tokens[index++];
                Require(command.Length == 1 && "MLHVZ".Contains(command), "Every path segment needs an explicit absolute command.");
                int count = command == "M" || command == "L" ? 2 : command == "Z" ? 0 : 1;
                Require(index + count <= tokens.Count, "Incomplete SVG path segment.");
                float a = count > 0 ? Number(tokens[index++]) : 0;
                float b = count > 1 ? Number(tokens[index++]) : 0;
                if (command == "M") {
                    path.StartFigure(); x = startX = a; y = startY = b; open = true;
                } else {
                    Require(open, "SVG path must start with M; Z must be followed by M.");
                    if (command == "Z") {
                        path.CloseFigure(); x = startX; y = startY; open = false;
                    } else {
                        float nextX = command == "V" ? x : a;
                        float nextY = command == "H" ? y : command == "V" ? a : b;
                        path.AddLine(x, y, nextX, nextY); x = nextX; y = nextY;
                    }
                }
            }
            Require(path.PointCount >= 2, "Empty SVG path.");
            return path;
        } catch { path.Dispose(); throw; }
    }
    static List<GraphicsPath> Read(string xml) {
        Require(xml.Length <= 16384, "SVG source exceeds 16 KiB.");
        var settings = new XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit, XmlResolver = null };
        var document = new XmlDocument { XmlResolver = null };
        using (var reader = XmlReader.Create(new StringReader(xml), settings)) document.Load(reader);
        foreach (XmlNode node in document.ChildNodes) {
            Require(node.NodeType == XmlNodeType.Element || node.NodeType == XmlNodeType.XmlDeclaration ||
                node.NodeType == XmlNodeType.Whitespace || node.NodeType == XmlNodeType.Comment, "Unsupported XML node.");
        }
        XmlElement root = document.DocumentElement;
        Require(root != null && root.Name == "svg" && root.NamespaceURI == Ns, "Expected SVG namespace/root.");
        Attributes(root, "xmlns", "viewBox", "fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin");
        Require(root.GetAttribute("viewBox") == "0 0 24 24" && root.GetAttribute("fill") == "none" &&
            root.GetAttribute("stroke") == "#000000" && root.GetAttribute("stroke-width") == "1.65" &&
            root.GetAttribute("stroke-linecap") == "round" && root.GetAttribute("stroke-linejoin") == "round",
            "Only the documented fixed navigation SVG style is supported.");
        Require(root.ChildNodes.Count <= 128, "Too many SVG children.");
        var paths = new List<GraphicsPath>();
        try {
            foreach (XmlNode node in root.ChildNodes) {
                if (node.NodeType == XmlNodeType.Whitespace || node.NodeType == XmlNodeType.Comment) continue;
                var element = node as XmlElement;
                Require(element != null && element.NamespaceURI == Ns, "Unsupported SVG child node.");
                if (element.Name == "title" || element.Name == "desc") {
                    Attributes(element);
                    foreach (XmlNode text in element.ChildNodes)
                        Require(text.NodeType == XmlNodeType.Text, "SVG metadata must be plain text.");
                    continue;
                }
                Require(!element.HasChildNodes, "Nested SVG elements are not supported.");
                GraphicsPath path;
                switch (element.Name) {
                    case "path": Attributes(element, "d"); path = Path(element.GetAttribute("d")); break;
                    case "rect": {
                        Attributes(element, "x", "y", "width", "height");
                        float x = Number(element.GetAttribute("x")), y = Number(element.GetAttribute("y"));
                        float w = Number(element.GetAttribute("width")), h = Number(element.GetAttribute("height"));
                        Require(w > 0 && h > 0 && x + w <= 24 && y + h <= 24, "Invalid rectangle.");
                        path = new GraphicsPath(); path.AddRectangle(new RectangleF(x, y, w, h)); break;
                    }
                    case "circle": {
                        Attributes(element, "cx", "cy", "r");
                        float x = Number(element.GetAttribute("cx")), y = Number(element.GetAttribute("cy"));
                        float r = Number(element.GetAttribute("r"));
                        Require(r > 0 && x - r >= 0 && y - r >= 0 && x + r <= 24 && y + r <= 24, "Invalid circle.");
                        path = new GraphicsPath(); path.AddEllipse(x - r, y - r, 2 * r, 2 * r); break;
                    }
                    default: throw new InvalidDataException("Unsupported SVG element: " + element.Name);
                }
                paths.Add(path);
            }
            Require(paths.Count > 0, "SVG contains no artwork.");
            return paths;
        } catch { foreach (var path in paths) path.Dispose(); throw; }
    }
    public static byte[] Generate(string xml) {
        var paths = Read(xml);
        try {
            using (var stream = new MemoryStream())
            using (var writer = new BinaryWriter(stream, Encoding.ASCII)) {
                writer.Write(Encoding.ASCII.GetBytes("NAV1"));
                foreach (int size in Sizes) {
                    const int samples = 4;
                    using (var image = new Bitmap(size * samples, size * samples, PixelFormat.Format32bppArgb)) {
                        using (var graphics = Graphics.FromImage(image))
                        using (var pen = new Pen(Color.Black, 1.65f)) {
                            graphics.Clear(Color.Transparent);
                            graphics.SmoothingMode = SmoothingMode.AntiAlias;
                            graphics.ScaleTransform(size * samples / 24f, size * samples / 24f);
                            pen.StartCap = pen.EndCap = LineCap.Round; pen.LineJoin = LineJoin.Round;
                            foreach (var path in paths) graphics.DrawPath(pen, path);
                        }
                        writer.Write((ushort)size);
                        writer.Write((uint)(size * size));
                        bool transparent = false, opaque = false, partial = false;
                        for (int y = 0; y < size; ++y) for (int x = 0; x < size; ++x) {
                            int alpha = 0;
                            for (int sy = 0; sy < samples; ++sy) for (int sx = 0; sx < samples; ++sx)
                                alpha += image.GetPixel(x * samples + sx, y * samples + sy).A;
                            byte coverage = (byte)((alpha + 8) / 16);
                            transparent |= coverage == 0; opaque |= coverage >= 240;
                            partial |= coverage > 0 && coverage < 240;
                            Require((x != 0 && y != 0 && x != size - 1 && y != size - 1) || coverage == 0,
                                "Artwork touches frame boundary; preserve transparent padding.");
                            writer.Write(coverage);
                        }
                        Require(transparent && opaque && partial, "Expected transparent, solid and antialiased pixels.");
                    }
                }
                writer.Flush(); return stream.ToArray();
            }
        } finally { foreach (var path in paths) path.Dispose(); }
    }
    public static void SelfTest() {
        const string head = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000000' stroke-width='1.65' stroke-linecap='round' stroke-linejoin='round'>";
        string[] invalid = {
            "<!DOCTYPE svg [<!ENTITY x SYSTEM 'file:///not-read'>]>" + head + "<title>&x;</title></svg>",
            head + "<image href='https://invalid.example/icon'/></svg>",
            head + "<path d='M 3 3 L 20 20' transform='scale(2)'/></svg>",
            head + "<path d='m 3 3 l 20 20'/></svg>",
            head + "<path d='M 3 3 C 4 4 5 5 6 6'/></svg>",
            head + "<path d='M 3 3 L 25 20'/></svg>",
            head + "<g><circle cx='12' cy='12' r='3'/></g></svg>",
            head + "<script>alert(1)</script></svg>",
            head + "<path d='M 3 3 L'/></svg>"
        };
        foreach (string xml in invalid) {
            bool rejected = false;
            try { Generate(xml); } catch (XmlException) { rejected = true; } catch (InvalidDataException) { rejected = true; }
            Require(rejected, "Restricted SVG validation accepted an unsupported source.");
        }
    }
}
'@

if ($SelfTest) {
    [NavigationMasks]::SelfTest()
    Write-Output 'Restricted SVG rejection tests passed (9 cases).'
}
$directory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\assets\navigation'))
foreach ($name in @('overview', 'analytics', 'logs', 'settings')) {
    $source = Join-Path $directory ($name + '.svg')
    if ((Get-Item -LiteralPath $source).Length -gt 16384) { throw "SVG source too large: $source" }
    $bytes = [NavigationMasks]::Generate([IO.File]::ReadAllText($source))
    $destination = Join-Path $directory ($name + '.mask')
    if ($ValidateOnly) {
        $existing = [IO.File]::ReadAllBytes($destination)
        if ([Convert]::ToBase64String($bytes) -cne [Convert]::ToBase64String($existing)) {
            throw "Stale or invalid navigation mask: $destination"
        }
        Write-Output "Validated $name : six dimensions, transparency, antialiasing and exact source-derived bytes ($($bytes.Length) bytes)."
    } else {
        [IO.File]::WriteAllBytes($destination, $bytes)
        Write-Output "Generated $name ($($bytes.Length) bytes)."
    }
}
