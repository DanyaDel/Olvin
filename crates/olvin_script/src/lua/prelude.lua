-- Runs once when the Lua state is created, before any script.
-- Gets the `ctx` methods table (Rust primitives that take and return plain
-- numbers) and adds the vector API on top of it, in Lua: building and
-- reading small tables is cheap here and costly through the Rust API.
-- Returns the `vec3` constructor (Rust uses it for vector values it hands
-- to scripts).
local M = ...

local type, error, setmetatable, sqrt = type, error, setmetatable, math.sqrt
local format = string.format

-- vec3 ---------------------------------------------------------------------

local V = {}
V.__index = V
V.__metatable = "vec3"

local function new(x, y, z)
    return setmetatable({ x = x, y = y, z = z }, V)
end

function vec3(x, y, z)
    return new(x or 0, y or 0, z or 0)
end

V.__add = function(a, b) return new(a.x + b.x, a.y + b.y, a.z + b.z) end
V.__sub = function(a, b) return new(a.x - b.x, a.y - b.y, a.z - b.z) end
V.__unm = function(a) return new(-a.x, -a.y, -a.z) end
V.__mul = function(a, b)
    if type(a) == "number" then return new(a * b.x, a * b.y, a * b.z) end
    if type(b) == "number" then return new(a.x * b, a.y * b, a.z * b) end
    return new(a.x * b.x, a.y * b.y, a.z * b.z)
end
V.__div = function(a, b)
    if type(b) == "number" then return new(a.x / b, a.y / b, a.z / b) end
    return new(a.x / b.x, a.y / b.y, a.z / b.z)
end
V.__eq = function(a, b) return a.x == b.x and a.y == b.y and a.z == b.z end
V.__tostring = function(a) return format("vec3(%g, %g, %g)", a.x, a.y, a.z) end

function V.dot(a, b) return a.x * b.x + a.y * b.y + a.z * b.z end
function V.length(a) return sqrt(a.x * a.x + a.y * a.y + a.z * a.z) end
function V.distance(a, b)
    local x, y, z = a.x - b.x, a.y - b.y, a.z - b.z
    return sqrt(x * x + y * y + z * z)
end
function V.normalized(a)
    local l = sqrt(a.x * a.x + a.y * a.y + a.z * a.z)
    if l < 1e-12 then return new(0, 0, 0) end
    return new(a.x / l, a.y / l, a.z / l)
end
function V.cross(a, b)
    return new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
end
function V.lerp(a, b, t)
    return new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t)
end
function V.unpack(a) return a.x, a.y, a.z end

-- Vector arguments: vec3(...), {x=, y=, z=}, {x, y, z} or three numbers.
-- Missing axes are 0. Errors point at the script's line (level 3).
local function xyz(v, y, z)
    if type(v) == "number" then return v, y or 0, z or 0 end
    if type(v) ~= "table" then
        error("expected a vector: vec3(x, y, z), {x=, y=, z=} or three numbers", 3)
    end
    return v.x or v[1] or 0, v.y or v[2] or 0, v.z or v[3] or 0
end

local function vec_or_nil(x, y, z)
    if x == nil then return nil end
    return new(x, y, z)
end

-- Getters: `name()` returns a vec3 built from `name_xyz()`.
local function getter(name)
    local raw = M[name .. "_xyz"]
    if raw then
        M[name] = function(ctx, ...) return vec_or_nil(raw(ctx, ...)) end
    end
end

-- Setters: `name(v)` or `name(x, y, z)` call `name_xyz(x, y, z)`.
local function setter(name)
    local raw = M[name .. "_xyz"]
    if raw then
        M[name] = function(ctx, v, y, z) return raw(ctx, xyz(v, y, z)) end
    end
end

-- Same for another entity: `name(id, v)` / `name(id, x, y, z)`.
local function setter_of(name)
    local raw = M[name .. "_xyz"]
    if raw then
        M[name] = function(ctx, id, v, y, z) return raw(ctx, id, xyz(v, y, z)) end
    end
end

for _, n in ipairs({
    "position", "world_position", "rotation_euler_deg", "scale",
    "forward", "right", "up", "velocity",
    "position_of", "world_position_of", "rotation_euler_deg_of", "scale_of",
}) do getter(n) end

for _, n in ipairs({
    "set_position", "translate", "translate_local", "rotate_euler_deg",
    "set_rotation_euler_deg", "set_scale",
    "apply_force", "apply_impulse", "set_velocity",
}) do setter(n) end

for _, n in ipairs({
    "set_position_of", "translate_of", "set_rotation_euler_deg_of", "set_scale_of",
}) do setter_of(n) end

do
    local raw = M.look_at_xyz
    -- look_at(target[, up]) or look_at(x, y, z)
    M.look_at = function(ctx, t, a, b)
        if type(t) == "number" then return raw(ctx, t, a or 0, b or 0) end
        local x, y, z = xyz(t)
        if a ~= nil then
            local ux, uy, uz = xyz(a)
            return raw(ctx, x, y, z, ux, uy, uz)
        end
        return raw(ctx, x, y, z)
    end
end

if M.axis2d_xy then
    local raw = M.axis2d_xy
    M.axis2d = function(ctx, action)
        local x, y = raw(ctx, action)
        return { x = x, y = y }
    end
end

if M.raycast_xyz then
    local raw = M.raycast_xyz
    -- raycast(origin, direction, max) -> nil or { entity, point, normal, distance }
    M.raycast = function(ctx, o, d, max)
        local ox, oy, oz = xyz(o)
        local dx, dy, dz = xyz(d)
        local e, px, py, pz, nx, ny, nz, dist = raw(ctx, ox, oy, oz, dx, dy, dz, max)
        if e == nil then return nil end
        return { entity = e, point = new(px, py, pz), normal = new(nx, ny, nz), distance = dist }
    end
end

if M.play_sound_at_xyz then
    local raw, where_of = M.play_sound_at_xyz, M.world_position_of_xyz
    -- play_sound_at(path, where[, options]): `where` is a vector or an entity id.
    M.play_sound_at = function(ctx, path, where, opts)
        if type(where) == "number" and math.type(where) == "integer" then
            local x, y, z = where_of(ctx, where)
            if x == nil then return nil end
            return raw(ctx, path, x, y, z, opts)
        end
        local x, y, z = xyz(where)
        return raw(ctx, path, x, y, z, opts)
    end
end

return vec3
