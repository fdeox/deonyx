// One full-window triangle; the pixel shader reads the captured window 1:1.
// A second pixel shader (photo) rates 4x4 blocks for looking like a photo;
// photos.rs turns those into rectangles kept in real colours.

Texture2D<float4> src : register(t0);

cbuffer Params : register(b0)
{
    float4 rects[32];  // protected areas in window pixels: left, top, right, bottom
    uint count;
    float lmin;        // lightness a white background becomes
    float lmax;        // lightness black text becomes
    float sat;         // chroma kept
    float split;       // > 0: pixels left of this x stay original (comparison view)
    float amount;      // 1 = darken, 0 = window is dark already, show it as it is
    float fringe;      // 1 = keep saturation, which calms ClearType colour fringes
    float holes;       // 1 = the layer can be see-through: kept areas show the real window
    float warm;        // 0..1 warm tint, like Night light
    float strength;    // 0..1: 0 shows the real colours, for fading in and out
    float boost;       // curve on the flipped lightness: < 1 lifts faint text
    float pad;
};

float4 vs(uint id : SV_VertexID) : SV_Position
{
    float2 p = float2((id << 1) & 2, id & 2);
    return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);
}

float3 to_lin(float3 c)
{
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}

float3 to_srgb(float3 c)
{
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

// OKLab (Bjorn Ottosson): lightness that matches what the eye sees.
float3 to_oklab(float3 c)
{
    float l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
    float m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
    float s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
    l = pow(max(l, 0), 1.0 / 3.0);
    m = pow(max(m, 0), 1.0 / 3.0);
    s = pow(max(s, 0), 1.0 / 3.0);
    return float3(
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s);
}

float3 from_oklab(float3 c)
{
    float l = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
    float m = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
    float s = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
    l = l * l * l;
    m = m * m * m;
    s = s * s * s;
    return float3(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
}

float3 lab_at(int2 p, int2 size)
{
    p = clamp(p, int2(0, 0), size - 1);
    return to_oklab(to_lin(src.Load(int3(p, 0)).rgb));
}

// Kept as it is: a hole in the layer when it can be see-through (the real,
// live window shows), else the captured pixel.
float4 keep(float4 c)
{
    return holes > 0.0 ? float4(0, 0, 0, 0) : float4(c.rgb, 1);
}

// The darkened colour, shown as strongly as `strength` says: over the real
// window (see-through, premultiplied) or mixed with the captured pixel.
float4 shown(float3 rgb, float4 c)
{
    return holes > 0.0 ? float4(rgb * strength, strength) : float4(lerp(c.rgb, rgb, strength), 1);
}

float4 ps(float4 pos : SV_Position) : SV_Target
{
    float4 c = src.Load(int3(pos.xy, 0));
    if (pos.x < split || amount <= 0.0)
        return keep(c);
    for (uint k = 0; k < count; k++)
    {
        float4 r = rects[k];
        if (pos.x >= r.x && pos.y >= r.y && pos.x < r.z && pos.y < r.w)
            return keep(c);
    }
    if (holes > 0.0)
    {
        // Protected video (DRM) reaches the capture as solid black. Flipped,
        // it would cover the film with grey; leave a hole so it plays. Real
        // black there looks the same either way, and text never fills a
        // whole 8x8 block with pure black.
        float3 block = src.Load(int3(int2(pos.xy) >> 3, 3)).rgb;
        if (max(c.r, max(c.g, c.b)) < 0.002 && max(block.r, max(block.g, block.b)) < 0.002)
            return float4(0, 0, 0, 0);
    }
    // Flip lightness only: white turns dark grey, black turns soft white,
    // and a red stays red.
    float3 lab = to_oklab(to_lin(c.rgb));
    if (fringe > 0.0)
    {
        // ClearType draws a black stroke as a row of tinted pixels (yellow,
        // dark red, dark blue, cyan) that the eye mixes back to black. Flipped
        // to light they come apart into red and blue lines. Five pixels whose
        // tints cancel out, across a strong light/dark step, are such text:
        // give the pixel their average colour, which is close to grey.
        // A single-hue line, an icon or a colour border doesn't cancel out.
        uint w, h, levels;
        src.GetDimensions(0, w, h, levels);
        int2 size = int2(w, h);
        int2 p = int2(pos.xy);
        float2 sum = 0;
        float chroma = 0;
        float lo = lab.x;
        float hi = lab.x;
        [unroll] for (int k = -2; k <= 2; k++)
        {
            float3 q = k == 0 ? lab : lab_at(p + int2(k, 0), size);
            sum += q.yz;
            chroma += length(q.yz);
            lo = min(lo, q.x);
            hi = max(hi, q.x);
        }
        float2 avg = sum / 5.0;
        float mean = chroma / 5.0;
        if (hi - lo > 0.4 && mean > 0.02 && length(avg) < 0.4 * mean)
            lab.yz = avg;
    }
    float before = lab.x;
    // Light grey text on white ends up close to the dark background; a curve
    // that lifts the low end keeps it readable while black text and the
    // background stay where they are.
    float t = saturate(1.0 - lab.x);
    if (boost > 0.0 && boost < 1.0)
        t = pow(t, boost);
    lab.x = lmin + t * (lmax - lmin);
    // Keep saturation (colour relative to lightness), not raw colour: a
    // faint pink beside white text, say a ClearType fringe, would otherwise
    // turn into a loud red outline once it is dark.
    float keep = fringe > 0.0 ? pow(saturate(lab.x / max(before, 0.001)), 0.8) : 1.0;
    lab.yz *= sat * keep;
    float3 rgb = to_srgb(saturate(from_oklab(lab)));
    rgb *= lerp(float3(1, 1, 1), float3(1.0, 0.86, 0.68), warm);
    return shown(rgb, c);
}

float light(float3 c)
{
    return pow(max(dot(to_lin(c), float3(0.2126, 0.7152, 0.0722)), 0), 1.0 / 3.0);
}

// One output pixel per 4x4 block of the window: 1 if the block looks like a
// photo or a video frame, 0.5 if it's at least not a flat fill (a calm sky
// inside a photo, say), 0 for flat colour. Interfaces are made of flat
// colour, so most pixels there equal a neighbour; text has two levels (paper
// and ink) with a few soft edge pixels between. A photo has neither: its
// neighbours differ and many of its pixels sit between its darkest and
// lightest.
float photo(float4 pos : SV_Position) : SV_Target
{
    uint w, h, levels;
    src.GetDimensions(0, w, h, levels);
    int2 size = int2(w, h);
    int2 o = int2(pos.xy) * 4;
    float l[16];
    float lo = 1, hi = 0;
    float flat = 0;
    float solid = 0;
    [unroll] for (int y = 0; y < 4; y++)
    {
        [unroll] for (int x = 0; x < 4; x++)
        {
            int2 p = clamp(o + int2(x, y), int2(0, 0), size - 1);
            float3 c = src.Load(int3(p, 0)).rgb;
            float3 r = src.Load(int3(clamp(p + int2(1, 0), int2(0, 0), size - 1), 0)).rgb;
            float3 d = src.Load(int3(clamp(p + int2(0, 1), int2(0, 0), size - 1), 0)).rgb;
            bool same_r = all(abs(c - r) < 0.5 / 255.0);
            bool same_d = all(abs(c - d) < 0.5 / 255.0);
            flat += (same_r || same_d) ? 1.0 : 0.0;
            solid += (same_r && same_d) ? 1.0 : 0.0;
            float v = light(c);
            l[y * 4 + x] = v;
            lo = min(lo, v);
            hi = max(hi, v);
        }
    }
    float range = hi - lo;
    float mid = 0;
    [unroll] for (int i = 0; i < 16; i++)
        mid += (l[i] > lo + 0.2 * range && l[i] < hi - 0.2 * range) ? 1.0 : 0.0;
    if (flat / 16.0 < 0.35 && mid / 16.0 >= 0.3 && range >= 0.05)
        return 1.0;
    return solid / 16.0 < 0.5 ? 0.5 : 0.0;
}
