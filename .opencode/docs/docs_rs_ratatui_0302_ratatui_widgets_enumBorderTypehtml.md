# BorderType in ratatui::widgets - Rust

> Source: https://docs.rs/ratatui/0.30.2/ratatui/widgets/enum.BorderType.html
> Cached: 2026-07-11T19:49:33.471Z

---

[ratatui](../index.html)::[widgets](index.html)# Enum BorderType Copy item path

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#36) ```
pub enum BorderType {
    Plain,
    Rounded,
    Double,
    Thick,
    LightDoubleDashed,
    HeavyDoubleDashed,
    LightTripleDashed,
    HeavyTripleDashed,
    LightQuadrupleDashed,
    HeavyQuadrupleDashed,
    QuadrantInside,
    QuadrantOutside,
}
```

Expand descriptionThe type of border of a [`Block`](struct.Block.html).

See the [`borders`](struct.Block.html#method.borders) method of `Block` to configure its borders.

## Variants[§](#variants)

[§](#variant.Plain)### Plain

A plain, simple border.

This is the default

#### [§](#example)Example

```
┌───────┐
│       │
└───────┘
```

[§](#variant.Rounded)### Rounded

A plain border with rounded corners.

#### [§](#example-1)Example

```
╭───────╮
│       │
╰───────╯
```

[§](#variant.Double)### Double

A doubled border.

Note this uses one character that draws two lines.

#### [§](#example-2)Example

```
╔═══════╗
║       ║
╚═══════╝
```

[§](#variant.Thick)### Thick

A thick border.

#### [§](#example-3)Example

```
┏━━━━━━━┓
┃       ┃
┗━━━━━━━┛
```

[§](#variant.LightDoubleDashed)### LightDoubleDashed

A light double-dashed border.

```
┌╌╌╌╌╌╌╌┐
╎       ╎
└╌╌╌╌╌╌╌┘
```

[§](#variant.HeavyDoubleDashed)### HeavyDoubleDashed

A heavy double-dashed border.

```
┏╍╍╍╍╍╍╍┓
╏       ╏
┗╍╍╍╍╍╍╍┛
```

[§](#variant.LightTripleDashed)### LightTripleDashed

A light triple-dashed border.

```
┌┄┄┄┄┄┄┄┐
┆       ┆
└┄┄┄┄┄┄┄┘
```

[§](#variant.HeavyTripleDashed)### HeavyTripleDashed

A heavy triple-dashed border.

```
┏┅┅┅┅┅┅┅┓
┇       ┇
┗┅┅┅┅┅┅┅┛
```

[§](#variant.LightQuadrupleDashed)### LightQuadrupleDashed

A light quadruple-dashed border.

```
┌┈┈┈┈┈┈┈┐
┊       ┊
└┈┈┈┈┈┈┈┘
```

[§](#variant.HeavyQuadrupleDashed)### HeavyQuadrupleDashed

A heavy quadruple-dashed border.

```
┏┉┉┉┉┉┉┉┓
┋       ┋
┗┉┉┉┉┉┉┉┛
```

[§](#variant.QuadrantInside)### QuadrantInside

A border with a single line on the inside of a half block.

#### [§](#example-4)Example

```
▗▄▄▄▄▄▄▄▖
▐       ▌
▐       ▌
▝▀▀▀▀▀▀▀▘
```

[§](#variant.QuadrantOutside)### QuadrantOutside

A border with a single line on the outside of a half block.

#### [§](#example-5)Example

```
▛▀▀▀▀▀▀▀▜
▌       ▐
▌       ▐
▙▄▄▄▄▄▄▄▟
```

## Implementations[§](#implementations)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#153)[§](#impl-BorderType)### impl [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#155)#### pub const fn [border_symbols](#method.border_symbols)<'a>(border_type: [BorderType](enum.BorderType.html)) -> [Set](../symbols/border/struct.Set.html)<'a>

Convert this `BorderType` into the corresponding [`Set`](../symbols/border/struct.Set.html) of border symbols.

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#173)#### pub const fn [to_border_set](#method.to_border_set)<'a>(self) -> [Set](../symbols/border/struct.Set.html)<'a>

Convert this `BorderType` into the corresponding [`Set`](../symbols/border/struct.Set.html) of border symbols.

## Trait Implementations[§](#trait-implementations)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Clone-for-BorderType)### impl [Clone](https://doc.rust-lang.org/nightly/core/clone/trait.Clone.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.clone)#### fn [clone](https://doc.rust-lang.org/nightly/core/clone/trait.Clone.html#tymethod.clone)(&self) -> [BorderType](enum.BorderType.html)

Returns a duplicate of the value. [Read more](https://doc.rust-lang.org/nightly/core/clone/trait.Clone.html#tymethod.clone)1.0.0 (const: [unstable](https://github.com/rust-lang/rust/issues/142757)) · [Source](https://doc.rust-lang.org/nightly/src/core/clone.rs.html#245-247)[§](#method.clone_from)#### fn [clone_from](https://doc.rust-lang.org/nightly/core/clone/trait.Clone.html#method.clone_from)(&mut self, source: &Self)

Performs copy-assignment from `source`. [Read more](https://doc.rust-lang.org/nightly/core/clone/trait.Clone.html#method.clone_from)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Copy-for-BorderType)### impl [Copy](https://doc.rust-lang.org/nightly/core/marker/trait.Copy.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Debug-for-BorderType)### impl [Debug](https://doc.rust-lang.org/nightly/core/fmt/trait.Debug.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.fmt)#### fn [fmt](https://doc.rust-lang.org/nightly/core/fmt/trait.Debug.html#tymethod.fmt)(&self, f: &mut [Formatter](https://doc.rust-lang.org/nightly/core/fmt/struct.Formatter.html)<'_>) -> [Result](https://doc.rust-lang.org/nightly/core/result/enum.Result.html)<[()](https://doc.rust-lang.org/nightly/std/primitive.unit.html), [Error](https://doc.rust-lang.org/nightly/core/fmt/struct.Error.html)>

Formats the value using the given formatter. [Read more](https://doc.rust-lang.org/nightly/core/fmt/trait.Debug.html#tymethod.fmt)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Default-for-BorderType)### impl [Default](https://doc.rust-lang.org/nightly/core/default/trait.Default.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.default)#### fn [default](https://doc.rust-lang.org/nightly/core/default/trait.Default.html#tymethod.default)() -> [BorderType](enum.BorderType.html)

Returns the “default value” for a type. [Read more](https://doc.rust-lang.org/nightly/core/default/trait.Default.html#tymethod.default)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#35)[§](#impl-Deserialize%3C'de%3E-for-BorderType)### impl<'de> [Deserialize](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/de/trait.Deserialize.html)<'de> for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#35)[§](#method.deserialize)fn [deserialize](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/de/trait.Deserialize.html#tymethod.deserialize)<__D>(
    __deserializer: __D,
) -> [Result](https://doc.rust-lang.org/nightly/core/result/enum.Result.html)<[BorderType](enum.BorderType.html), <__D as [Deserializer](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/de/trait.Deserializer.html)<'de>>::[Error](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/de/trait.Deserializer.html#associatedtype.Error)>where
    __D: [Deserializer](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/de/trait.Deserializer.html)<'de>,Deserialize this value from the given Serde deserializer. [Read more](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/de/trait.Deserialize.html#tymethod.deserialize)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Display-for-BorderType)### impl [Display](https://doc.rust-lang.org/nightly/core/fmt/trait.Display.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.fmt-1)#### fn [fmt](https://doc.rust-lang.org/nightly/core/fmt/trait.Display.html#tymethod.fmt)(&self, f: &mut [Formatter](https://doc.rust-lang.org/nightly/core/fmt/struct.Formatter.html)<'_>) -> [Result](https://doc.rust-lang.org/nightly/core/result/enum.Result.html)<[()](https://doc.rust-lang.org/nightly/std/primitive.unit.html), [Error](https://doc.rust-lang.org/nightly/core/fmt/struct.Error.html)>

Formats the value using the given formatter. [Read more](https://doc.rust-lang.org/nightly/core/fmt/trait.Display.html#tymethod.fmt)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Eq-for-BorderType)### impl [Eq](https://doc.rust-lang.org/nightly/core/cmp/trait.Eq.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-FromStr-for-BorderType)### impl [FromStr](https://doc.rust-lang.org/nightly/core/str/traits/trait.FromStr.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#associatedtype.Err)#### type [Err](https://doc.rust-lang.org/nightly/core/str/traits/trait.FromStr.html#associatedtype.Err) = [ParseError](https://docs.rs/strum/0.28.0/x86_64-unknown-linux-gnu/strum/enum.ParseError.html)

The associated error which can be returned from parsing.[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.from_str)#### fn [from_str](https://doc.rust-lang.org/nightly/core/str/traits/trait.FromStr.html#tymethod.from_str)(s: &[str](https://doc.rust-lang.org/nightly/std/primitive.str.html)) -> [Result](https://doc.rust-lang.org/nightly/core/result/enum.Result.html)<[BorderType](enum.BorderType.html), <[BorderType](enum.BorderType.html) as [FromStr](https://doc.rust-lang.org/nightly/core/str/traits/trait.FromStr.html)>::[Err](https://doc.rust-lang.org/nightly/core/str/traits/trait.FromStr.html#associatedtype.Err)>

Parses a string `s` to return a value of this type. [Read more](https://doc.rust-lang.org/nightly/core/str/traits/trait.FromStr.html#tymethod.from_str)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-Hash-for-BorderType)### impl [Hash](https://doc.rust-lang.org/nightly/core/hash/trait.Hash.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.hash)fn [hash](https://doc.rust-lang.org/nightly/core/hash/trait.Hash.html#tymethod.hash)<__H>(&self, state: [&mut __H](https://doc.rust-lang.org/nightly/std/primitive.reference.html))where
    __H: [Hasher](https://doc.rust-lang.org/nightly/core/hash/trait.Hasher.html),Feeds this value into the given [`Hasher`](https://doc.rust-lang.org/nightly/core/hash/trait.Hasher.html). [Read more](https://doc.rust-lang.org/nightly/core/hash/trait.Hash.html#tymethod.hash)1.3.0 · [Source](https://doc.rust-lang.org/nightly/src/core/hash/mod.rs.html#234-236)[§](#method.hash_slice)fn [hash_slice](https://doc.rust-lang.org/nightly/core/hash/trait.Hash.html#method.hash_slice)<H>(data: &[Self], state: [&mut H](https://doc.rust-lang.org/nightly/std/primitive.reference.html))where
    H: [Hasher](https://doc.rust-lang.org/nightly/core/hash/trait.Hasher.html),
    Self: [Sized](https://doc.rust-lang.org/nightly/core/marker/trait.Sized.html),Feeds a slice of this type into the given [`Hasher`](https://doc.rust-lang.org/nightly/core/hash/trait.Hasher.html). [Read more](https://doc.rust-lang.org/nightly/core/hash/trait.Hash.html#method.hash_slice)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-PartialEq-for-BorderType)### impl [PartialEq](https://doc.rust-lang.org/nightly/core/cmp/trait.PartialEq.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#method.eq)#### fn [eq](https://doc.rust-lang.org/nightly/core/cmp/trait.PartialEq.html#tymethod.eq)(&self, other: &[BorderType](enum.BorderType.html)) -> [bool](https://doc.rust-lang.org/nightly/std/primitive.bool.html)

Tests for `self` and `other` values to be equal, and is used by `==`.1.0.0 (const: [unstable](https://github.com/rust-lang/rust/issues/143800)) · [Source](https://doc.rust-lang.org/nightly/src/core/cmp.rs.html#263)[§](#method.ne)#### fn [ne](https://doc.rust-lang.org/nightly/core/cmp/trait.PartialEq.html#method.ne)(&self, other: [&Rhs](https://doc.rust-lang.org/nightly/std/primitive.reference.html)) -> [bool](https://doc.rust-lang.org/nightly/std/primitive.bool.html)

Tests for `!=`. The default implementation is almost always sufficient,
and should not be overridden without very good reason.[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#35)[§](#impl-Serialize-for-BorderType)### impl [Serialize](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serialize.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#35)[§](#method.serialize)fn [serialize](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serialize.html#tymethod.serialize)<__S>(
    &self,
    __serializer: __S,
) -> [Result](https://doc.rust-lang.org/nightly/core/result/enum.Result.html)<<__S as [Serializer](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serializer.html)>::[Ok](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serializer.html#associatedtype.Ok), <__S as [Serializer](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serializer.html)>::[Error](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serializer.html#associatedtype.Error)>where
    __S: [Serializer](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serializer.html),Serialize this value into the given Serde serializer. [Read more](https://docs.rs/serde_core/1.0.228/x86_64-unknown-linux-gnu/serde_core/ser/trait.Serialize.html#tymethod.serialize)[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-StructuralPartialEq-for-BorderType)### impl [StructuralPartialEq](https://doc.rust-lang.org/nightly/core/marker/trait.StructuralPartialEq.html) for [BorderType](enum.BorderType.html)

[Source](https://docs.rs/ratatui-widgets/0.3.2/x86_64-unknown-linux-gnu/src/ratatui_widgets/borders.rs.html#34)[§](#impl-TryFrom%3C%26str%3E-for-BorderType)### impl [TryFrom](https://doc.rust-lang.org/nightly/core/convert

... [Content truncated]