-- Spinning obstacle (a sweeper bar, a rotating floor): turns its entity around
-- one of its own axes. Give the entity a Kinematic Rigidbody and a collider and
-- it shoves the character controller out of the way as it turns.
return {
    params = {
        { name = "speed", kind = "float", default = 60, doc = "degrees per second (negative: the other way)" },
        { name = "axis", kind = "string", default = "y", doc = "x, y or z (its own axis)" },
    },

    update = function(self, ctx)
        local a = self.params.speed * ctx:dt()
        local axis = self.params.axis
        if axis == "x" then
            ctx:rotate_euler_deg_xyz(a, 0, 0)
        elseif axis == "z" then
            ctx:rotate_euler_deg_xyz(0, 0, a)
        else
            ctx:rotate_euler_deg_xyz(0, a, 0)
        end
    end,
}
