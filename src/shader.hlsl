// One full-window triangle; the pixel shader reads the captured window 1:1.

Texture2D<float4> src : register(t0);

cbuffer Params : register(b0)
{
    float4 rects[8];   // protected areas in window pixels: left, top, right, bottom
    uint count;
    float lmin;        // lightness a white background becomes
    float lmax;        // lightness black text becomes
    float sat;         // chroma kept
    float split;       // > 0: pixels left of this x stay original (comparison view)
    float amount;      // 1 = darken, 0 = window is dark already, show it as it is
    float2 pad;
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

float4 ps(float4 pos : SV_Position) : SV_Target
{
    float4 c = src.Load(int3(pos.xy, 0));
    if (pos.x < split || amount <= 0.0)
        return float4(c.rgb, 1);
    for (uint k = 0; k < count; k++)
    {
        float4 r = rects[k];
        if (pos.x >= r.x && pos.y >= r.y && pos.x < r.z && pos.y < r.w)
            return float4(c.rgb, 1);
    }
    // Flip lightness only: white turns dark grey, black turns soft white,
    // and a red stays red.
    float3 lab = to_oklab(to_lin(c.rgb));
    lab.x = lmin + (1.0 - lab.x) * (lmax - lmin);
    lab.yz *= sat;
    return float4(to_srgb(saturate(from_oklab(lab))), 1);
}
