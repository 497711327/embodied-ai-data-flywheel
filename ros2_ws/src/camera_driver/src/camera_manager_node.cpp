void CameraManagerNode::loadCameras()
{
    this->declare_parameter("cameras", "");

    auto yaml = this->get_parameter("cameras")
                    .as_string();

    // 解析 YAML（简化逻辑）
    for (auto & cam : parsed_yaml)
    {
        auto node = std::make_shared<CameraDriverNode>(
            options,
            cam.device_id,
            cam.topic,
            cam.frame_id,
            cam.fps
        );

        cameras_.push_back(node);
    }
}