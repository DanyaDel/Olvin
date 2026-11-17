use crate::App;

/// A self-contained engine module. A plugin registers its resources and systems
/// in [`Plugin::build`]; it talks to other plugins only through shared resources.
pub trait Plugin: 'static {
    fn build(&self, app: &mut App);

    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}
