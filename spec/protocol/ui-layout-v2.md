# OpenMat UI layout extension v2

This additive extension augments `ui-v1.md`, `ui-class-v2.md` and
`ui-composite-v3.md`. It does not change callback ownership or the UI MIME's
version. The API is OpenMat-specific, not a claim of MATLAB App Designer parity.

## XML capability marker

An `OpenMatUI` root of document version 1, 2 or 3 may additionally carry
`layoutVersion="2"`. With this marker, Layout accepts the optional fields below
in addition to all previously defined fields. A missing marker permits only
legacy layout fields. Unsupported marker values and unknown layout attributes
are errors. Canonical serialization emits the marker whenever an extended field
is explicitly stored, or a ScrollPanel primitive is present; otherwise it omits
the marker. Existing fields are not implicitly rewritten to new defaults.

```xml
<OpenMatUI version="2" class="LayoutLab" layoutVersion="2">
  <Component id="root" name="MainWindow" type="Window">
    <Layout mode="grid" columnTracks="260 1fr" rowTracks="auto 1fr 150" />
  </Component>
</OpenMatUI>
```

| XML / snapshot key | Native Layout field | Effective default | Meaning |
| --- | --- | --- | --- |
| widthMode / heightMode | WidthMode / HeightMode | `auto` | `auto`, `fixed`, `content`, `fill` |
| minWidth / minHeight | MinWidth / MinHeight | `0` | Minimum rendered size in pixels |
| maxWidth / maxHeight | MaxWidth / MaxHeight | `10000` | Maximum rendered size in pixels |
| growX / growY | GrowX / GrowY | `1` | Flex main-axis fill weight |
| rowTracks / columnTracks | RowTracks / ColumnTracks | empty string | Explicit grid track sizes |

All numbers are finite and within [0,10000]. Maxima must be at least 1 and no
minimum may exceed its corresponding maximum. Legacy positive integer index,
span and count rules remain. An absent constraint retains the previous
renderer behavior; default values are semantic fallbacks, not mandatory fields
in a persisted document or M Layout struct.

Track lists contain at most 64 whitespace-separated entries. Each entry is
lowercase `auto` or an unsigned decimal matching `digits[.digits]` followed by
optional `px` or `fr`. The numeric value must be in (0,10000]. Plain numbers mean
pixels. Percentages, signs, exponents, CSS functions and declarations are errors.
An empty column list uses `Columns` equal tracks. An explicit column list sets
the effective column count. An explicit row list uses those tracks followed by
implicit `minmax(min-content, auto)` rows; an empty row list preserves the prior
equal flexible implicit rows. Track `fr` follows CSS grid's intrinsic minimum
behavior; zero minimum item sizes allow shrinking where specified.

## Rendering and precedence

`auto` retains the previous parent-specific behavior: absolute coordinates and
dimensions (or nonempty native Position), grid stretch/minimum Height, and flex
Width/Height/Grow. `fixed` uses Width or Height and prevents main-axis flex growth;
`content` uses intrinsic maximum-content size with no main-axis growth; `fill`
stretches within the grid cell, distributes free space in flex, or sets equal
opposing X/Y margins in absolute layout. Each explicit mode supplies a zero
minimum unless a minimum is specified. Explicit bounds apply to every mode.

GrowX/GrowY apply only to the corresponding main axis of a row/column parent and
only in fill mode. Cross-axis fill stretches; fixed/content align to the start.
Grid fill uses the available cell and its track sizing; fractions on tracks
control grid distribution. Fixed sizes and declared minima can intentionally
overflow a parent; ScrollPanel controls which axes are scrollable.

Position retains legacy absolute-parent precedence for coordinates. Explicit
size modes then apply to dimensions. Layout.X/Y retain top-left design units,
whereas native Position retains bottom-left units. Grid and flex placement ignore
Position. Composite host overrides may include sizing fields, but internal
Mode/Columns/Gap/Padding/RowTracks/ColumnTracks belong to the component definition.

## Native and presentation boundary

`openmat.ui.ScrollPanel` derives from ComponentContainer and has
`ScrollDirection = 'vertical'`; supported renderer values are `vertical`,
`horizontal` and `both`. It owns children with the existing lifecycle and has a
column layout by default.

Native Layout fields map to lower-initial JSON keys in the existing optional
`layout` snapshot/describe object. The runtime validates the complete struct when
publishing. Invalid constraints produce a native error; they do not cross as raw
CSS. Unknown fields remain errors. The updated client reads old snapshots; new
fields require the updated client and backend. Older clients can reject new
payloads, since no capability negotiation or old-client downgrade is introduced.

`application/vnd.openmat.ui+json` remains version 1 and retains its existing node,
text and payload limits. Runtime layout updates affect only the running renderer,
not the saved definition. IDs, class names, component references and callback
bindings retain their previous version-specific contracts.
